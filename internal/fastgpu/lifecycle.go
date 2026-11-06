package fastgpu

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"

	"golang.org/x/term"
)

func choose(labels []string) (int, error) {
	if !term.IsTerminal(int(os.Stdin.Fd())) {
		return 0, fmt.Errorf("ambiguous selection without a terminal: specify an ID or path")
	}
	for i, l := range labels {
		fmt.Fprintf(os.Stderr, "%d) %s\n", i+1, l)
	}
	fmt.Fprint(os.Stderr, "Selection: ")
	s, err := bufio.NewReader(os.Stdin).ReadString('\n')
	if err != nil {
		return 0, err
	}
	i, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil || i < 1 || i > len(labels) {
		return 0, fmt.Errorf("invalid selection")
	}
	return i - 1, nil
}
func (a *App) selectVolume(ctx context.Context, c *API, selector string) (Volume, error) {
	v, err := c.volumes(ctx)
	if err != nil {
		return Volume{}, err
	}
	if selector != "" {
		return resolveVolume(v, selector)
	}
	if a.Config.Volume != "" {
		return resolveVolume(v, a.Config.Volume)
	}
	if len(v) == 0 {
		return Volume{}, fmt.Errorf("no Network Volume: use fastgpu volumes create")
	}
	if len(v) == 1 {
		return v[0], nil
	}
	labels := make([]string, len(v))
	for i, x := range v {
		labels[i] = fmt.Sprintf("%s (%s) — %d GB, %s", x.Name, x.ID, x.Size, x.DataCenter)
	}
	i, err := choose(labels)
	if err != nil {
		return Volume{}, err
	}
	return v[i], nil
}
func (a *App) offers(ctx context.Context, c *API, v Volume, size string) ([]GPU, error) {
	q := url.Values{"include": {"AVAILABILITY"}, "product": {"POD"}, "count": {"1"}, "cloud": {"SECURE"}, "minCudaVersion": {a.Config.MinCUDA}}
	var r struct {
		GPUs []GPU `json:"gpus"`
	}
	if err := c.request(ctx, "GET", "/catalog/gpus?"+q.Encode(), nil, &r); err != nil {
		return nil, err
	}
	g := eligible(r.GPUs, v.DataCenter, a.Config.MinVRAM, a.Config.MaxPrice)
	g, err := sizeOffers(g, size)
	if err != nil {
		return nil, err
	}
	if len(g) == 0 {
		return nil, fmt.Errorf("no compatible %s On-Demand NVIDIA GPU available in %s within the size's price range and configured price cap (VRAM >= %d GB, CUDA >= %s)", size, v.DataCenter, a.Config.MinVRAM, a.Config.MinCUDA)
	}
	return g, nil
}
func (a *App) awaitReady(ctx context.Context, c *API, id string) (Pod, error) {
	ctx, cancel := context.WithTimeout(ctx, 25*time.Minute)
	defer cancel()
	last := ""
	for {
		p, err := c.pod(ctx, id)
		if err != nil {
			return p, err
		}
		if p.Status != last {
			fmt.Fprintf(os.Stderr, "Pod %s: %s\n", id, p.Status)
			last = p.Status
		}
		if p.Status == "EXITED" || p.Status == "TERMINATED" {
			return p, fmt.Errorf("Pod %s stopped during startup", id)
		}
		if p.Status == "RUNNING" && p.SSH.Direct != nil {
			var output strings.Builder
			err = a.remote(ctx, p, []string{"test", "-f", "/run/gpu/ready"}, nil, &output, &output, false)
			if err == nil {
				return p, nil
			}
		}
		if err = sleep(ctx, 5*time.Second); err != nil {
			return p, fmt.Errorf("Pod %s not ready: %w; check fastgpu status and fastgpu logs --pod, or fastgpu down", id, err)
		}
	}
}
func (a *App) up(ctx context.Context, selector, size string, dry, skipModel bool) (Pod, error) {
	c, err := a.client()
	if err != nil {
		return Pod{}, err
	}
	if a.State.PodID != "" {
		p, e := c.pod(ctx, a.State.PodID)
		if e == nil && p.Status != "TERMINATED" {
			if selector != "" {
				v, e := a.selectVolume(ctx, c, selector)
				if e != nil {
					return p, e
				}
				if !mounted(p, v.ID) {
					return p, fmt.Errorf("Pod %s already exists and uses another volume; run fastgpu down before creating a new machine", p.ID)
				}
			}
			fmt.Fprintf(os.Stderr, "Pod %s already exists (%s); reusing this machine. Run fastgpu down before creating a new machine or changing its size.\n", p.ID, p.Status)
			if dry {
				return p, printJSON(p)
			}
			if p.Status == "EXITED" {
				if e = c.request(ctx, "POST", "/pods/"+url.PathEscape(p.ID)+"/action", map[string]string{"action": "start"}, nil); e != nil {
					return p, e
				}
			}
			p, e = a.awaitReady(ctx, c, p.ID)
			if e != nil {
				return p, e
			}
			if !skipModel {
				e = a.remote(ctx, p, []string{"fastgpu", "worker", "prepare"}, nil, os.Stdout, os.Stderr, false)
			}
			return p, e
		}
		if e != nil && !isStatus(e, 404) {
			return Pod{}, e
		}
		a.State = State{}
		if err = a.saveState(); err != nil {
			return Pod{}, err
		}
	}
	if a.State.PendingName != "" {
		pods, e := c.pods(ctx)
		if e != nil {
			return Pod{}, e
		}
		var found []Pod
		for _, p := range pods {
			if p.Name == a.State.PendingName && p.Status != "TERMINATED" {
				found = append(found, p)
			}
		}
		if len(found) != 1 {
			return Pod{}, fmt.Errorf("previous creation has an uncertain outcome (%s): found %d Pods. Check fastgpu pods; use fastgpu use ID or fastgpu recover --clear only after verification", a.State.PendingName, len(found))
		}
		a.State.PodID = found[0].ID
		a.State.Managed = true
		a.State.PendingName = ""
		if e = a.saveState(); e != nil {
			return Pod{}, e
		}
		return a.up(ctx, selector, size, dry, skipModel)
	}
	pods, err := c.pods(ctx)
	if err != nil {
		return Pod{}, err
	}
	for _, p := range pods {
		if strings.HasPrefix(p.Name, "gpu-") && p.Status != "TERMINATED" {
			return Pod{}, fmt.Errorf("Pod %s already exists (%s); only one fastgpu machine is allowed. Run fastgpu down from its profile, or recover it with fastgpu use %s before running fastgpu down", p.ID, p.Status, p.ID)
		}
	}
	v, err := a.selectVolume(ctx, c, selector)
	if err != nil {
		return Pod{}, err
	}
	g, err := a.offers(ctx, c, v, size)
	if err != nil {
		return Pod{}, err
	}
	if a.Config.Disk < 20 || a.Config.Context < 512 || a.Config.MinVRAM < 1 || a.Config.MinRAM < 1 || a.Config.Image == "" {
		return Pod{}, fmt.Errorf("invalid configuration: check fastgpu config")
	}
	fmt.Fprintf(os.Stderr, "Volume %s (%s), %s; size %s; GPU %s, %d GB VRAM; On-Demand list price $%.3f/h + storage\n", v.Name, v.ID, v.DataCenter, size, g[0].Name, g[0].Memory, g[0].Price.Secure)
	if dry {
		return Pod{}, printJSON(map[string]any{"volume": v, "size": size, "offers": g, "image": a.Config.Image, "disk_gb": a.Config.Disk, "min_ram_gb": a.Config.MinRAM})
	}
	if err = a.ensureKey(ctx); err != nil {
		return Pod{}, err
	}
	pub, err := os.ReadFile(a.Config.SSHKey + ".pub")
	if err != nil {
		return Pod{}, err
	}
	a.State = State{PendingName: randomName(), VolumeID: v.ID, Managed: true}
	if err = a.saveState(); err != nil {
		return Pod{}, err
	}
	body := map[string]any{"name": a.State.PendingName, "image": a.Config.Image, "cloud": "SECURE", "dataCenterIds": []string{v.DataCenter}, "disk": a.Config.Disk, "ports": []string{"22/tcp"}, "env": map[string]string{"PUBLIC_KEY": strings.TrimSpace(string(pub)), "GPU_WORKSPACE_ID": a.State.PendingName}, "mounts": map[string]any{"network": []Mount{{v.ID, workspace}}}}
	if a.Config.Registry != "" {
		body["registry"] = a.Config.Registry
	}
	var p Pod
	for i, offer := range g {
		body["gpu"] = map[string]any{"id": offer.ID, "count": 1, "minRamPerGpu": a.Config.MinRAM, "minVcpuCountPerGpu": 4, "minCudaVersion": a.Config.MinCUDA}
		err = c.request(ctx, "POST", "/pods", body, &p)
		if err == nil {
			break
		}
		// Retry a different GPU only after an explicit capacity rejection, never after an ambiguous timeout.
		if capacityRejected(err) && i+1 < len(g) {
			fmt.Fprintln(os.Stderr, "Capacity exhausted; trying", g[i+1].Name)
			continue
		}
		if isStatus(err, 400) || isStatus(err, 401) || isStatus(err, 403) || isStatus(err, 422) {
			a.State = State{}
			_ = a.saveState()
		}
		return Pod{}, fmt.Errorf("creation: %w (operation %s)", err, body["name"])
	}
	if p.ID == "" {
		return p, fmt.Errorf("creation response has no ID; check fastgpu pods before retrying")
	}
	a.State.PodID = p.ID
	a.State.PendingName = ""
	if err = a.saveState(); err != nil {
		return p, fmt.Errorf("Pod %s created but saving local state failed: %w", p.ID, err)
	}
	fmt.Fprintf(os.Stderr, "Created %s; actual compute cost $%.3f/h. To terminate it: fastgpu down\n", p.ID, p.Cost)
	if a.Config.MaxPrice > 0 && p.Cost > a.Config.MaxPrice {
		err = c.request(ctx, "DELETE", "/pods/"+url.PathEscape(p.ID), nil, nil)
		if err != nil {
			return p, fmt.Errorf("cost exceeds the limit and termination failed: %w", err)
		}
		a.State = State{}
		_ = a.saveState()
		return p, fmt.Errorf("actual cost exceeds --max-price; Pod terminated, volume retained")
	}
	p, err = a.awaitReady(ctx, c, p.ID)
	if err != nil {
		return p, err
	}
	if !skipModel {
		err = a.remote(ctx, p, []string{"fastgpu", "worker", "prepare"}, nil, os.Stdout, os.Stderr, false)
	}
	return p, err
}

func capacityRejected(err error) bool {
	if !isStatus(err, 400) {
		return false
	}
	detail := strings.ToLower(err.Error())
	return strings.Contains(detail, "capacity") || strings.Contains(detail, "no longer any instances available")
}
func mounted(p Pod, id string) bool {
	for _, m := range p.Mounts.Network {
		if m.VolumeID == id {
			return true
		}
	}
	return false
}
func printJSON(v any) error {
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	return e.Encode(v)
}
func (a *App) down(ctx context.Context) error {
	c, err := a.client()
	if err != nil {
		return err
	}
	p, err := a.selected(ctx, c)
	if isStatus(err, 404) {
		a.State = State{}
		return a.saveState()
	}
	if err != nil {
		return err
	}
	if !a.State.Managed || len(p.Mounts.Network) != 1 || !mounted(p, a.State.VolumeID) {
		return fmt.Errorf("termination refused: Pod identity/storage does not match the managed state")
	}
	a.closeTunnel(ctx, p)
	fmt.Fprintf(os.Stderr, "Terminating Pod %s; Network Volume %s retained\n", p.ID, a.State.VolumeID)
	if err = c.request(ctx, "DELETE", "/pods/"+url.PathEscape(p.ID), nil, nil); err != nil {
		return err
	}
	wait, cancel := context.WithTimeout(ctx, 2*time.Minute)
	defer cancel()
	for {
		p, e := c.pod(wait, p.ID)
		if isStatus(e, 404) || (e == nil && p.Status == "TERMINATED") {
			break
		}
		if e != nil {
			return e
		}
		if e = sleep(wait, 2*time.Second); e != nil {
			return e
		}
	}
	var v Volume
	if err = c.request(ctx, "GET", "/network-volumes/"+url.PathEscape(a.State.VolumeID), nil, &v); err != nil {
		return fmt.Errorf("Pod terminated; volume verification failed: %w", err)
	}
	a.State = State{}
	if err = a.saveState(); err != nil {
		return err
	}
	fmt.Printf("Pod terminated. Volume %s (%s), %d GB, is still present.\n", v.Name, v.ID, v.Size)
	return nil
}
