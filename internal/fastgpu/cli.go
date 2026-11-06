package fastgpu

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/spf13/cobra"
)

func Execute(ctx context.Context) error {
	a, err := newApp()
	if err != nil {
		return err
	}
	defer a.release()
	return a.command(ctx).Execute()
}

func (a *App) command(ctx context.Context) *cobra.Command {
	root := &cobra.Command{Use: "fastgpu", Short: "Your remote GPU environment on RunPod", Version: Version, SilenceUsage: true, SilenceErrors: true}
	add := func(use, help string, n int, mutate bool, fn func(*cobra.Command, []string) error) *cobra.Command {
		c := &cobra.Command{Use: use, Short: help, Args: cobra.ExactArgs(n), RunE: func(c *cobra.Command, args []string) error {
			if mutate {
				if e := a.acquire(); e != nil {
					return e
				}
			}
			return fn(c, args)
		}}
		root.AddCommand(c)
		return c
	}
	initCmd := add("init", "Configure the image, SSH key, and volume", 0, true, func(c *cobra.Command, _ []string) error {
		api, e := a.client()
		if e != nil {
			return e
		}
		sel, _ := c.Flags().GetString("volume")
		v, e := a.selectVolume(ctx, api, sel)
		if e != nil {
			return e
		}
		a.Config.Volume = v.ID
		if e = a.ensureKey(ctx); e != nil {
			return e
		}
		if e = a.saveConfig(); e != nil {
			return e
		}
		fmt.Println("Configuration saved:", filepath.Join(a.Dir, "config.json"))
		return printJSON(a.Config)
	})
	initCmd.Flags().StringVar(&a.Config.Image, "image", a.Config.Image, "Docker image (digest recommended)")
	initCmd.Flags().StringVar(&a.Config.SSHKey, "ssh-key", a.Config.SSHKey, "SSH private key")
	initCmd.Flags().StringVar(&a.Config.Registry, "registry", a.Config.Registry, "RunPod registry credential ID")
	initCmd.Flags().String("volume", "", "Network Volume ID or name")
	add("config", "Show configuration without credentials", 0, false, func(_ *cobra.Command, _ []string) error { return printJSON(a.Config) })
	up := add("up", "Create or resume one Pod (default size: small)", 0, true, func(c *cobra.Command, _ []string) error {
		v, _ := c.Flags().GetString("volume")
		dry, _ := c.Flags().GetBool("dry-run")
		skip, _ := c.Flags().GetBool("skip-model")
		size := "small"
		for _, candidate := range []string{"small", "medium", "large", "xlarge"} {
			if selected, _ := c.Flags().GetBool(candidate); selected {
				size = candidate
			}
		}
		_, e := a.up(ctx, v, size, dry, skip)
		return e
	})
	up.Flags().Bool("small", false, "Cheapest compatible GPU below $1.50/hour (default)")
	up.Flags().Bool("medium", false, "GPU closest to $3/hour, from $1.50 to below $3.50/hour")
	up.Flags().Bool("large", false, "Cheapest compatible GPU from $3.50 to $5/hour")
	up.Flags().Bool("xlarge", false, "Cheapest compatible GPU above $5/hour")
	up.MarkFlagsMutuallyExclusive("small", "medium", "large", "xlarge")
	up.Flags().String("volume", "", "Volume ID or name (override)")
	up.Flags().Bool("dry-run", false, "Show the plan without creating resources")
	up.Flags().Bool("skip-model", false, "Skip the GGUF download")
	up.Flags().Float64Var(&a.Config.MaxPrice, "max-price", a.Config.MaxPrice, "Maximum compute price in USD/hour")
	up.Flags().IntVar(&a.Config.MinVRAM, "min-vram", a.Config.MinVRAM, "Minimum VRAM in GB")
	up.Flags().StringVar(&a.Config.Image, "image", a.Config.Image, "Image for this startup")
	up.Flags().IntVar(&a.Config.Disk, "disk", a.Config.Disk, "Container disk size in GB")
	root.Args = cobra.NoArgs
	root.RunE = func(_ *cobra.Command, _ []string) error { return up.RunE(up, nil) }
	add("pods", "List Pods without modifying them", 0, false, func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := c.pods(ctx)
		if e != nil {
			return e
		}
		return printJSON(p)
	})
	add("status", "Show Pod details, status, and compute rate", 0, false, func(_ *cobra.Command, _ []string) error {
		if a.State.PodID == "" {
			return printJSON(a.State)
		}
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		return printJSON(map[string]any{"pod": p, "local": a.State})
	})
	add("use ID", "Select a Pod with the fastgpu image and a Network Volume", 1, true, func(_ *cobra.Command, args []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := c.pod(ctx, args[0])
		if e != nil {
			return e
		}
		if !strings.HasPrefix(p.Name, "gpu-") || len(p.Mounts.Network) != 1 {
			return fmt.Errorf("fastgpu use only accepts fastgpu Pods with a Network Volume")
		}
		if _, e = a.remoteOutput(ctx, p, "fastgpu", "--version"); e != nil {
			return fmt.Errorf("could not verify the fastgpu image: %w", e)
		}
		a.State = State{PodID: p.ID, VolumeID: p.Mounts.Network[0].VolumeID, Managed: true}
		return a.saveState()
	})
	add("down", "Terminate the managed Pod and retain the Network Volume", 0, true, func(_ *cobra.Command, _ []string) error { return a.down(ctx) })
	add("stop", "Stop the Pod (volume billing continues)", 0, true, func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		if !a.State.Managed {
			return fmt.Errorf("Pod is not managed")
		}
		a.closeTunnel(ctx, p)
		if e = c.request(ctx, "POST", "/pods/"+url.PathEscape(p.ID)+"/action", map[string]string{"action": "stop"}, nil); e != nil {
			return e
		}
		return a.saveState()
	})
	recoverCmd := add("recover", "Clear an uncertain creation after manual verification", 0, true, func(c *cobra.Command, _ []string) error {
		clear, _ := c.Flags().GetBool("clear")
		if !clear {
			return printJSON(a.State)
		}
		if a.State.PodID != "" {
			return fmt.Errorf("a Pod is selected: use down")
		}
		api, e := a.client()
		if e != nil {
			return e
		}
		pods, e := api.pods(ctx)
		if e != nil {
			return e
		}
		for _, p := range pods {
			if p.Name == a.State.PendingName {
				return fmt.Errorf("Pod still exists: fastgpu use %s", p.ID)
			}
		}
		a.State = State{}
		return a.saveState()
	})
	recoverCmd.Flags().Bool("clear", false, "Confirm that you also checked the console for the absent Pod")
	shell := add("shell [command...]", "Open an interactive shell or run a remote command", 0, false, func(c *cobra.Command, args []string) error {
		api, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, api)
		if e != nil {
			return e
		}
		cwd, _ := c.Flags().GetString("cwd")
		r, e := remotePath(cwd)
		if e != nil {
			return e
		}
		tty, _ := c.Flags().GetBool("tty")
		if len(args) == 0 {
			args = []string{"bash", "-l"}
			tty = true
		}
		script := "cd -- " + quote(r) + " && exec " + shellWords(args)
		return a.remote(ctx, p, []string{"bash", "-lc", script}, os.Stdin, os.Stdout, os.Stderr, tty)
	})
	shell.Args = cobra.ArbitraryArgs
	shell.Flags().SetInterspersed(false)
	shell.Flags().String("cwd", "home:", "Remote directory with home: or vol:")
	shell.Flags().Bool("tty", false, "Allocate a terminal for the command")
	add("mkdir DIRECTORY", "Create a persistent directory in the home directory or volume", 1, false, func(_ *cobra.Command, args []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		r, e := remotePath(args[0])
		if e != nil {
			return e
		}
		return a.remote(ctx, p, []string{"mkdir", "-p", "--", r}, nil, os.Stdout, os.Stderr, false)
	})
	code := add("code [DIRECTORY]", "Open a remote directory in local VS Code", 0, false, func(_ *cobra.Command, args []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		folder := ""
		if len(args) == 1 {
			folder = args[0]
		}
		return a.openCode(ctx, p, folder)
	})
	code.Args = cobra.MaximumNArgs(1)
	for _, direction := range []string{"push", "pull"} {
		direction := direction
		add(direction+" SOURCE DESTINATION", "Copy with rsync without deleting destination files", 2, false, func(_ *cobra.Command, args []string) error {
			c, e := a.client()
			if e != nil {
				return e
			}
			p, e := a.selected(ctx, c)
			if e != nil {
				return e
			}
			if direction == "push" {
				return a.transfer(ctx, p, args[0], args[1], true)
			}
			return a.transfer(ctx, p, args[1], args[0], false)
		})
	}
	model := &cobra.Command{Use: "model", Short: "Manage persistent GGUF models"}
	root.AddCommand(model)
	model.AddCommand(&cobra.Command{Use: "pull [qwen3.8-27b]", Short: "Download and verify the GGUF", Args: validModel, RunE: func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		return a.remote(ctx, p, []string{"fastgpu", "worker", "prepare"}, nil, os.Stdout, os.Stderr, false)
	}})
	model.AddCommand(&cobra.Command{Use: "list", Short: "List model files on the volume", Args: cobra.NoArgs, RunE: func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		return a.remote(ctx, p, []string{"ls", "-lh", modelDir}, nil, os.Stdout, os.Stderr, false)
	}})
	var deployment Deployment
	deploy := add("deploy [MODEL_OR_PATH]", "Serve a GGUF or checkpoint with llama.cpp, Ollama, or vLLM", 0, true, func(_ *cobra.Command, args []string) error {
		d := deployment
		d.Context = a.Config.Context
		d.Target = ModelAlias
		if len(args) == 1 {
			d.Target = args[0]
		}
		if d.Model == "" {
			d.Model = defaultModelName(d.Target)
		}
		if d.Target != ModelAlias && !d.HF {
			var e error
			d.Target, e = modelPath(d.Target)
			if e != nil {
				return e
			}
		}
		if d.Modelfile != "" {
			var e error
			d.Modelfile, e = modelPath(d.Modelfile)
			if e != nil {
				return e
			}
		}
		if strings.HasPrefix(d.Tokenizer, "/") || strings.HasPrefix(d.Tokenizer, "home:") || strings.HasPrefix(d.Tokenizer, "vol:") {
			var e error
			d.Tokenizer, e = modelPath(d.Tokenizer)
			if e != nil {
				return e
			}
		}
		if e := validateDeployment(d); e != nil {
			return e
		}
		p, e := a.up(ctx, "", "small", false, true)
		if e != nil {
			return e
		}
		version, e := a.remoteOutput(ctx, p, "fastgpu", "--version")
		if e != nil {
			return e
		}
		if strings.TrimSpace(version) != "fastgpu version "+Version {
			return fmt.Errorf("remote CLI version mismatch (%s): use an image built with fastgpu %s; terminate the old Pod first with fastgpu down", strings.TrimSpace(version), Version)
		}
		body, _ := json.Marshal(d)
		if e = a.remote(ctx, p, []string{"fastgpu", "worker", "deploy"}, bytes.NewReader(body), os.Stdout, os.Stderr, false); e != nil {
			return e
		}
		wait, cancel := context.WithTimeout(ctx, 20*time.Minute)
		defer cancel()
		for {
			var ignored strings.Builder
			e = a.remote(wait, p, []string{"fastgpu", "worker", "ready"}, nil, &ignored, &ignored, false)
			if e == nil {
				break
			}
			if e = sleep(wait, 3*time.Second); e != nil {
				return fmt.Errorf("model not ready: check fastgpu logs; the Pod remains running")
			}
		}
		if e = a.tunnel(ctx, p, a.Config.LocalPort); e != nil {
			return e
		}
		if e = readyModel(ctx, fmt.Sprintf("http://127.0.0.1:%d", a.Config.LocalPort), d); e != nil {
			return e
		}
		fmt.Printf("Engine: %s\nModel: %s\nBase URL: http://127.0.0.1:%d/v1\n", d.Engine, d.Model, a.Config.LocalPort)
		return nil
	})
	deploy.Args = cobra.MaximumNArgs(1)
	deploy.Flags().StringVar(&deployment.Engine, "engine", "llama.cpp", "Engine: llama.cpp, ollama, vllm")
	deploy.Flags().StringVar(&deployment.Model, "name", "", "Model name in the local API")
	deploy.Flags().BoolVar(&deployment.HF, "hf", false, "Treat the model as a Hugging Face ID (vLLM)")
	deploy.Flags().StringVar(&deployment.Tokenizer, "tokenizer", "", "Hugging Face tokenizer or remote path (vLLM)")
	deploy.Flags().StringVar(&deployment.Modelfile, "modelfile", "", "Custom Modelfile on the volume (Ollama)")
	deploy.Flags().StringArrayVar(&deployment.Args, "arg", nil, "Additional engine argument; repeatable, e.g. --arg=--enforce-eager")
	deploy.Flags().IntVar(&a.Config.MinVRAM, "min-vram", a.Config.MinVRAM, "Minimum VRAM for a new Pod")
	deploy.Flags().IntVar(&a.Config.Context, "context", a.Config.Context, "Context size in tokens")
	deploy.Flags().IntVar(&a.Config.LocalPort, "local-port", a.Config.LocalPort, "Local tunnel port")
	deploy.Flags().Float64Var(&a.Config.MaxPrice, "max-price", a.Config.MaxPrice, "Maximum compute price in USD/hour")
	add("undeploy", "Stop inference; the Pod remains running", 0, true, func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		a.closeTunnel(ctx, p)
		if e = a.saveState(); e != nil {
			return e
		}
		return a.remote(ctx, p, []string{"fastgpu", "worker", "stop"}, nil, os.Stdout, os.Stderr, false)
	})
	tunnel := add("tunnel", "Open or close the model tunnel", 0, true, func(c *cobra.Command, _ []string) error {
		api, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, api)
		if e != nil {
			return e
		}
		closeIt, _ := c.Flags().GetBool("close")
		if closeIt {
			a.closeTunnel(ctx, p)
			return a.saveState()
		}
		return a.tunnel(ctx, p, a.Config.LocalPort)
	})
	tunnel.Flags().IntVar(&a.Config.LocalPort, "local-port", a.Config.LocalPort, "Local port")
	tunnel.Flags().Bool("close", false, "Close the tunnel")
	logs := add("logs", "Show model or container logs", 0, false, func(cmd *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		podLogs, _ := cmd.Flags().GetBool("pod")
		if podLogs {
			return c.logs(ctx, p.ID, os.Stdout)
		}
		follow, _ := cmd.Flags().GetBool("follow")
		args := []string{"tail", "-n", "100"}
		if follow {
			args = append(args, "-F")
		}
		args = append(args, "/workspace/.gpu/logs/inference.log")
		return a.remote(ctx, p, args, nil, os.Stdout, os.Stderr, false)
	})
	logs.Flags().Bool("follow", false, "Follow logs")
	logs.Flags().Bool("pod", false, "RunPod container logs")
	doctor := add("doctor", "Check dependencies and remote installations", 0, false, func(c *cobra.Command, _ []string) error {
		for _, name := range []string{"ssh", "rsync"} {
			p, e := exec.LookPath(name)
			if e != nil {
				return e
			}
			fmt.Println(name, p)
		}
		if p, e := exec.LookPath("code"); e == nil {
			fmt.Println("code", p)
		} else {
			fmt.Println("VS Code not found (optional; required by fastgpu code)")
		}
		api, e := a.client()
		if e != nil {
			return e
		}
		v, e := api.volumes(ctx)
		if e != nil {
			return e
		}
		fmt.Printf("RunPod API OK; %d Network Volumes\n", len(v))
		remote, _ := c.Flags().GetBool("remote")
		if remote {
			p, e := a.selected(ctx, api)
			if e != nil {
				return e
			}
			return a.remote(ctx, p, []string{"fastgpu", "worker", "check"}, nil, os.Stdout, os.Stderr, false)
		}
		return nil
	})
	doctor.Flags().Bool("remote", false, "Check programs and CUDA computation in the Pod")
	add("cost", "Show the compute rate and duration (not a bill)", 0, false, func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		p, e := a.selected(ctx, c)
		if e != nil {
			return e
		}
		return printJSON(map[string]any{"pod": p.ID, "status": p.Status, "compute_usd_per_hour": p.Cost, "created_at": p.CreatedAt, "note": "Storage is billed separately; the compute rate does not imply billing while EXITED"})
	})
	a.volumeCommands(root, ctx)
	worker := &cobra.Command{Use: "worker ACTION", Hidden: true, Args: cobra.ExactArgs(1), RunE: func(_ *cobra.Command, args []string) error {
		switch args[0] {
		case "prepare":
			return prepareModel(ctx)
		case "serve":
			return workerServe()
		case "check":
			return workerCheck(ctx)
		case "ready":
			return workerReady(ctx)
		case "stop":
			return supervisor(ctx, "stop")
		case "deploy":
			var d Deployment
			decoder := json.NewDecoder(io.LimitReader(os.Stdin, 1<<20))
			decoder.DisallowUnknownFields()
			if e := decoder.Decode(&d); e != nil {
				return e
			}
			return workerDeploy(ctx, d)
		default:
			return fmt.Errorf("unknown worker action")
		}
	}}
	root.AddCommand(worker)
	a.infoCommands(root, ctx)
	configureHelp(root)
	root.SetContext(ctx)
	return root
}
func validModel(_ *cobra.Command, args []string) error {
	if len(args) > 1 || (len(args) == 1 && args[0] != ModelAlias) {
		return fmt.Errorf("supported model: %s", ModelAlias)
	}
	return nil
}

func (a *App) volumeCommands(root *cobra.Command, ctx context.Context) {
	list := func(_ *cobra.Command, _ []string) error {
		c, e := a.client()
		if e != nil {
			return e
		}
		v, e := c.volumes(ctx)
		if e != nil {
			return e
		}
		return printJSON(v)
	}
	vol := &cobra.Command{Use: "volumes", Aliases: []string{"volume"}, Short: "View and manage Network Volumes", Args: cobra.NoArgs, RunE: list}
	root.AddCommand(vol)
	vol.AddCommand(&cobra.Command{Use: "list", Args: cobra.NoArgs, RunE: list})
	add := func(use string, n int, fn func(*API, Volume, []string, *cobra.Command) error) *cobra.Command {
		c := &cobra.Command{Use: use, Args: cobra.ExactArgs(n), RunE: func(cmd *cobra.Command, args []string) error {
			if e := a.acquire(); e != nil {
				return e
			}
			api, e := a.client()
			if e != nil {
				return e
			}
			v, e := api.volumes(ctx)
			if e != nil {
				return e
			}
			x, e := resolveVolume(v, args[0])
			if e != nil {
				return e
			}
			return fn(api, x, args, cmd)
		}}
		vol.AddCommand(c)
		return c
	}
	add("show ID_OR_NAME", 1, func(_ *API, v Volume, _ []string, _ *cobra.Command) error { return printJSON(v) })
	add("use ID_OR_NAME", 1, func(_ *API, v Volume, _ []string, _ *cobra.Command) error {
		if a.State.PodID != "" && a.State.VolumeID != v.ID {
			return fmt.Errorf("the selected Pod uses another volume; run fastgpu down first")
		}
		a.Config.Volume = v.ID
		return a.saveConfig()
	})
	add("rename ID_OR_NAME NEW_NAME", 2, func(c *API, v Volume, args []string, _ *cobra.Command) error {
		if strings.TrimSpace(args[1]) == "" {
			return fmt.Errorf("empty name")
		}
		var out Volume
		if e := c.request(ctx, "PATCH", "/network-volumes/"+url.PathEscape(v.ID), map[string]string{"name": args[1]}, &out); e != nil {
			return e
		}
		return printJSON(out)
	})
	add("resize ID_OR_NAME GB", 2, func(c *API, v Volume, args []string, _ *cobra.Command) error {
		n, e := strconv.Atoi(args[1])
		if e != nil || n < v.Size || n > 4096 {
			return fmt.Errorf("size must be between %d and 4096 GB; shrinking is not allowed", v.Size)
		}
		var out Volume
		if e = c.request(ctx, "PATCH", "/network-volumes/"+url.PathEscape(v.ID), map[string]int{"size": n}, &out); e != nil {
			return e
		}
		return printJSON(out)
	})
	del := add("delete ID_OR_NAME", 1, func(c *API, v Volume, _ []string, cmd *cobra.Command) error {
		confirm, _ := cmd.Flags().GetString("confirm")
		if confirm != v.ID {
			return fmt.Errorf("irreversible operation: specify --confirm %s", v.ID)
		}
		if v.ID == a.Config.Volume || v.ID == a.State.VolumeID {
			return fmt.Errorf("protected volume: selected by the profile or managed Pod")
		}
		pods, e := c.pods(ctx)
		if e != nil {
			return e
		}
		for _, p := range pods {
			if mounted(p, v.ID) {
				return fmt.Errorf("volume attached to Pod %s; deletion refused", p.ID)
			}
		}
		return c.request(ctx, "DELETE", "/network-volumes/"+url.PathEscape(v.ID), nil, nil)
	})
	del.Flags().String("confirm", "", "Confirm by entering the exact volume ID")
	create := &cobra.Command{Use: "create NAME", Short: "Create a Network Volume (paid storage)", Args: cobra.ExactArgs(1), RunE: func(cmd *cobra.Command, args []string) error {
		if e := a.acquire(); e != nil {
			return e
		}
		size, _ := cmd.Flags().GetInt("size")
		dc, _ := cmd.Flags().GetString("datacenter")
		tier, _ := cmd.Flags().GetString("type")
		if size < 10 || size > 4096 || dc == "" || (tier != "STANDARD" && tier != "HIGH_PERFORMANCE") {
			return fmt.Errorf("specify --datacenter, --size between 10 and 4096, and --type STANDARD or HIGH_PERFORMANCE")
		}
		c, e := a.client()
		if e != nil {
			return e
		}
		var out Volume
		e = c.request(ctx, "POST", "/network-volumes", map[string]any{"name": args[0], "size": size, "dataCenter": dc, "type": tier}, &out)
		if e != nil {
			return e
		}
		return printJSON(out)
	}}
	create.Flags().Int("size", 0, "Size in GB")
	create.Flags().String("datacenter", "", "Data center")
	create.Flags().String("type", "STANDARD", "Storage tier")
	vol.AddCommand(create)
	for _, cmd := range vol.Commands() {
		if cmd.Short != "" {
			continue
		}
		cmd.Short = map[string]string{"list": "List Network Volumes", "show": "Show volume details", "use": "Select the profile volume", "rename": "Rename a volume", "resize": "Increase volume size", "delete": "Delete an unprotected volume with ID confirmation"}[cmd.Name()]
	}
}
