package fastgpu

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"sort"
	"strings"
	"time"
)

func (c *API) logs(ctx context.Context, id string, out io.Writer) error {
	r, err := http.NewRequestWithContext(ctx, "GET", c.Base+"/pods/"+url.PathEscape(id)+"/logs?tail=100", nil)
	if err != nil {
		return err
	}
	r.Header.Set("Authorization", "Bearer "+c.Key)
	r.Header.Set("Accept", "text/event-stream")
	h := *c.HTTP
	h.Timeout = 0
	resp, err := h.Do(r)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return &APIError{resp.StatusCode, http.StatusText(resp.StatusCode)}
	}
	s := bufio.NewScanner(resp.Body)
	s.Buffer(make([]byte, 4096), 1<<20)
	for s.Scan() {
		if !strings.HasPrefix(s.Text(), "data:") {
			continue
		}
		var entry struct {
			Source string `json:"source"`
			Line   string `json:"line"`
		}
		if err = json.Unmarshal([]byte(strings.TrimSpace(strings.TrimPrefix(s.Text(), "data:"))), &entry); err != nil {
			return err
		}
		if _, err = fmt.Fprintf(out, "[%s] %s\n", entry.Source, entry.Line); err != nil {
			return err
		}
	}
	return s.Err()
}

type API struct {
	Base, Key string
	HTTP      *http.Client
}
type APIError struct {
	Status int
	Detail string
}

func (e *APIError) Error() string    { return fmt.Sprintf("RunPod HTTP %d: %s", e.Status, e.Detail) }
func isStatus(err error, n int) bool { var e *APIError; return errors.As(err, &e) && e.Status == n }
func client() (*API, error) {
	k, e := apiKey()
	if e != nil {
		return nil, e
	}
	return &API{Base: "https://api.runpod.io/v2", Key: k, HTTP: &http.Client{Timeout: 45 * time.Second, CheckRedirect: func(_ *http.Request, _ []*http.Request) error { return http.ErrUseLastResponse }}}, nil
}

func (c *API) request(ctx context.Context, method, path string, body, out any) error {
	var data []byte
	var err error
	if body != nil {
		data, err = json.Marshal(body)
		if err != nil {
			return err
		}
	}
	for attempt := 0; attempt < 3; attempt++ {
		r, err := http.NewRequestWithContext(ctx, method, c.Base+path, bytes.NewReader(data))
		if err != nil {
			return err
		}
		r.Header.Set("Authorization", "Bearer "+c.Key)
		r.Header.Set("Content-Type", "application/json")
		resp, err := c.HTTP.Do(r)
		if err != nil {
			return err
		}
		b, readErr := io.ReadAll(io.LimitReader(resp.Body, 8<<20))
		resp.Body.Close()
		if readErr != nil {
			return readErr
		}
		if resp.StatusCode >= 200 && resp.StatusCode < 300 {
			if out != nil && len(b) > 0 {
				return json.Unmarshal(b, out)
			}
			return nil
		}
		if method == http.MethodGet && (resp.StatusCode == 429 || resp.StatusCode >= 500) && attempt < 2 {
			if err = sleep(ctx, time.Duration(attempt+1)*time.Second); err != nil {
				return err
			}
			continue
		}
		var detail struct {
			Detail string `json:"detail"`
			Title  string `json:"title"`
		}
		_ = json.Unmarshal(b, &detail)
		if detail.Detail == "" {
			detail.Detail = detail.Title
		}
		if detail.Detail == "" {
			detail.Detail = http.StatusText(resp.StatusCode)
		}
		return &APIError{resp.StatusCode, strings.ReplaceAll(detail.Detail, c.Key, "[redacted]")}
	}
	return fmt.Errorf("RunPod: retries exhausted")
}
func sleep(ctx context.Context, d time.Duration) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-time.After(d):
		return nil
	}
}

type Volume struct {
	ID         string `json:"id"`
	Name       string `json:"name"`
	Size       int    `json:"size"`
	DataCenter string `json:"dataCenter"`
	Type       string `json:"type"`
}
type Mount struct {
	VolumeID string `json:"volumeId"`
	Path     string `json:"path"`
}
type Endpoint struct {
	Host     string `json:"host"`
	Port     int    `json:"port"`
	Username string `json:"username"`
}
type Pod struct {
	ID         string  `json:"id"`
	Name       string  `json:"name"`
	Status     string  `json:"status"`
	Image      string  `json:"image"`
	Cost       float64 `json:"cost"`
	DataCenter string  `json:"dataCenterId"`
	CreatedAt  string  `json:"createdAt"`
	Mounts     struct {
		Network []Mount `json:"network"`
	} `json:"mounts"`
	SSH struct {
		Direct *Endpoint `json:"direct"`
	} `json:"ssh"`
	GPU struct {
		ID     string `json:"id"`
		Count  int    `json:"count"`
		Memory int    `json:"memory"`
	} `json:"gpu"`
}
type GPU struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	Manufacturer string `json:"manufacturer"`
	Memory       int    `json:"memory"`
	Secure       bool   `json:"secure"`
	Price        struct {
		Secure float64 `json:"secure"`
	} `json:"price"`
	DataCenters []struct {
		ID           string `json:"id"`
		Availability string `json:"availability"`
	} `json:"dataCenters"`
}

func (c *API) list(ctx context.Context, path, key string, out any) error {
	var items []json.RawMessage
	cursor := ""
	seen := map[string]bool{}
	for {
		p := path
		if cursor != "" {
			p += "?cursor=" + url.QueryEscape(cursor)
		}
		var r map[string]json.RawMessage
		if err := c.request(ctx, "GET", p, nil, &r); err != nil {
			return err
		}
		var page []json.RawMessage
		if err := json.Unmarshal(r[key], &page); err != nil {
			return err
		}
		items = append(items, page...)
		var pagination struct {
			NextCursor  string `json:"nextCursor"`
			HasNextPage bool   `json:"hasNextPage"`
		}
		if b := r["pagination"]; len(b) > 0 {
			if err := json.Unmarshal(b, &pagination); err != nil {
				return err
			}
		}
		if !pagination.HasNextPage {
			break
		}
		cursor = pagination.NextCursor
		if cursor == "" || seen[cursor] {
			return fmt.Errorf("invalid RunPod pagination")
		}
		seen[cursor] = true
	}
	b, err := json.Marshal(items)
	if err != nil {
		return err
	}
	return json.Unmarshal(b, out)
}
func (c *API) volumes(ctx context.Context) ([]Volume, error) {
	var v []Volume
	e := c.list(ctx, "/network-volumes", "networkVolumes", &v)
	return v, e
}
func (c *API) pods(ctx context.Context) ([]Pod, error) {
	var v []Pod
	e := c.list(ctx, "/pods", "pods", &v)
	return v, e
}
func (c *API) pod(ctx context.Context, id string) (Pod, error) {
	var p Pod
	e := c.request(ctx, "GET", "/pods/"+url.PathEscape(id), nil, &p)
	return p, e
}
func resolveVolume(v []Volume, selector string) (Volume, error) {
	var matches []Volume
	for _, x := range v {
		if x.ID == selector {
			return x, nil
		}
		if x.Name == selector {
			matches = append(matches, x)
		}
	}
	if len(matches) == 1 {
		return matches[0], nil
	}
	if len(matches) > 1 {
		return Volume{}, fmt.Errorf("ambiguous volume name: use the ID")
	}
	return Volume{}, fmt.Errorf("volume %q not found", selector)
}
func eligible(gpus []GPU, dc string, minVRAM int, maxPrice float64) []GPU {
	var out []GPU
	for _, g := range gpus {
		if !g.Secure || g.Manufacturer != "NVIDIA" || g.Memory < minVRAM || g.Price.Secure <= 0 || (maxPrice > 0 && g.Price.Secure > maxPrice) {
			continue
		}
		if !supportedArchitecture(g.ID) {
			continue
		}
		for _, d := range g.DataCenters {
			if d.ID == dc && d.Availability != "NONE" && d.Availability != "" {
				out = append(out, g)
				break
			}
		}
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Price.Secure == out[j].Price.Secure {
			return out[i].ID < out[j].ID
		}
		return out[i].Price.Secure < out[j].Price.Secure
	})
	return out
}

// sizeOffers preserves price order, except medium prefers offers closest to $3/hour.
func sizeOffers(gpus []GPU, size string) ([]GPU, error) {
	var minPrice, maxPrice float64
	switch size {
	case "small":
		maxPrice = 1.5
	case "medium":
		minPrice, maxPrice = 1.5, 3.5
	case "large":
		minPrice, maxPrice = 3.5, 5
	case "xlarge":
		minPrice = 5
	default:
		return nil, fmt.Errorf("unknown GPU size %q", size)
	}
	var out []GPU
	for _, g := range gpus {
		price := g.Price.Secure
		if price < minPrice || (size == "xlarge" && price == minPrice) {
			continue
		}
		if maxPrice > 0 && (price > maxPrice || (size != "large" && price == maxPrice)) {
			continue
		}
		out = append(out, g)
	}
	if size == "medium" {
		sort.SliceStable(out, func(i, j int) bool {
			return math.Abs(out[i].Price.Secure-3) < math.Abs(out[j].Price.Secure-3)
		})
	}
	return out, nil
}

// The image contains CUDA kernels for Ampere and newer GPUs, not Volta/Turing.
func supportedArchitecture(id string) bool {
	for _, legacy := range []string{"V100", "P100", "P40", "K80", "T4", "RTX 8000"} {
		if strings.Contains(id, legacy) {
			return false
		}
	}
	for _, name := range []string{"RTX 4000", "RTX 5000", "RTX 6000"} {
		if strings.Contains(id, name) && !strings.Contains(id, "Ada") && !strings.Contains(id, "PRO") {
			return false
		}
	}
	return true
}
