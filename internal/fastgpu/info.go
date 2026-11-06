package fastgpu

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"net/url"
	"path/filepath"
	"strconv"

	"github.com/spf13/cobra"
)

func publicPodDetails(raw map[string]any) map[string]any {
	out := map[string]any{}
	// Allowlist: environment, startup arguments, and registry credentials may contain secrets.
	for _, key := range []string{"id", "name", "status", "image", "cloud", "dataCenterId", "cudaVersion", "gpu", "cpu", "disk", "mounts", "ports", "ssh", "runtime", "cost", "createdAt", "startedAt", "actions", "locked", "globalNetworking", "template"} {
		if value, ok := raw[key]; ok {
			out[key] = value
		}
	}
	return out
}

func (a *App) connections(p Pod, local, remote int) (map[string]string, error) {
	if local < 1 || local > 65535 || remote < 1 || remote > 65535 {
		return nil, fmt.Errorf("allowed ports: 1..65535")
	}
	if p.Status != "RUNNING" || p.SSH.Direct == nil {
		return map[string]string{"status": "direct SSH unavailable: the Pod must be running with port 22/tcp published"}, nil
	}
	if err := a.sshConfig(p); err != nil {
		return nil, err
	}
	e := p.SSH.Direct
	config := filepath.Join(a.Dir, "ssh_config")
	host := "gpu-" + p.ID
	manual := []string{"ssh", "-tt", "-i", a.Config.SSHKey, "-p", strconv.Itoa(e.Port), "-o", "IdentitiesOnly=yes", "-o", "StrictHostKeyChecking=accept-new", "-o", "HostKeyAlias=" + host, "-o", "UserKnownHostsFile=" + filepath.Join(a.Dir, "known_hosts"), "gpu@" + e.Host}
	commands := map[string]string{
		"tcp":            net.JoinHostPort(e.Host, strconv.Itoa(e.Port)),
		"ssh":            shellWords(manual),
		"ssh_config":     shellWords([]string{"ssh", "-F", config, "-tt", host}),
		"tunnel_tcp":     shellWords([]string{"ssh", "-F", config, "-N", "-o", "ExitOnForwardFailure=yes", "-L", fmt.Sprintf("127.0.0.1:%d:127.0.0.1:%d", local, remote), host}),
		"local_endpoint": net.JoinHostPort("127.0.0.1", strconv.Itoa(local)),
	}
	if remote == 8080 {
		commands["local_api"] = fmt.Sprintf("http://127.0.0.1:%d/v1", local)
	}
	return commands, nil
}

func (a *App) infoCommands(root *cobra.Command, ctx context.Context) {
	var live bool
	info := &cobra.Command{Use: "info", Short: "Show hardware, storage, status, networking, and connection commands", Args: cobra.NoArgs, RunE: func(_ *cobra.Command, _ []string) error {
		api, err := a.client()
		if err != nil {
			return err
		}
		if a.State.PodID == "" {
			return fmt.Errorf("no Pod selected: use fastgpu up")
		}
		var raw map[string]any
		if err = api.request(ctx, "GET", "/pods/"+url.PathEscape(a.State.PodID), nil, &raw); err != nil {
			return err
		}
		b, err := json.Marshal(raw)
		if err != nil {
			return err
		}
		var p Pod
		if err = json.Unmarshal(b, &p); err != nil {
			return err
		}
		conn, err := a.connections(p, a.Config.LocalPort, 8080)
		if err != nil {
			return err
		}
		volumes := []Volume{}
		for _, mount := range p.Mounts.Network {
			var v Volume
			if err = api.request(ctx, "GET", "/network-volumes/"+url.PathEscape(mount.VolumeID), nil, &v); err != nil {
				return err
			}
			volumes = append(volumes, v)
		}
		report := map[string]any{"pod": publicPodDetails(raw), "volumes": volumes, "connections": conn, "persistence": map[string]string{"mount": workspace, "home": remoteHome + " -> /workspace/.gpu/home", "repository": "/home/gpu/progetti", "models": modelDir, "huggingface_cache": "/home/gpu/.cache/huggingface", "ollama": "/workspace/.gpu/ollama/models"}, "note": "API gpu.memory is system RAM, not VRAM; compute rate is USD/hour, storage is billed separately"}
		if live {
			output, e := a.remoteOutput(ctx, p, "python", "-c", `import json,os,platform,subprocess
from pathlib import Path
gpu=subprocess.run(['nvidia-smi','--query-gpu=name,memory.total,memory.used,utilization.gpu,driver_version','--format=csv'],capture_output=True,text=True)
print(json.dumps({'system':platform.platform(),'visible_cpus':os.cpu_count(),'resolved_home':os.path.realpath('/home/gpu'),'gpu_csv':gpu.stdout.strip(),'gpu_error':gpu.stderr.strip(),'kernel_memory':Path('/proc/meminfo').read_text().splitlines()[:3]}))`)
			if e != nil {
				return e
			}
			var details any
			if e = json.Unmarshal([]byte(output), &details); e != nil {
				return e
			}
			report["remote_system"] = details
		}
		return printJSON(report)
	}}
	info.Flags().BoolVar(&live, "live", false, "Also read GPU and operating system details over SSH")
	root.AddCommand(info)
	var asJSON bool
	local, remote := a.Config.LocalPort, 8080
	connect := &cobra.Command{Use: "connect", Short: "Print the TCP endpoint, SSH command, and tunnel command", Args: cobra.NoArgs, RunE: func(_ *cobra.Command, _ []string) error {
		api, err := a.client()
		if err != nil {
			return err
		}
		p, err := a.selected(ctx, api)
		if err != nil {
			return err
		}
		c, err := a.connections(p, local, remote)
		if err != nil {
			return err
		}
		if asJSON {
			return printJSON(c)
		}
		for _, key := range []string{"status", "tcp", "ssh", "ssh_config", "tunnel_tcp", "local_endpoint", "local_api"} {
			if v := c[key]; v != "" {
				fmt.Printf("%s: %s\n", key, v)
			}
		}
		return nil
	}}
	connect.Flags().BoolVar(&asJSON, "json", false, "JSON output for scripts")
	connect.Flags().IntVar(&local, "local-port", local, "Local port in the tunnel example")
	connect.Flags().IntVar(&remote, "remote-port", remote, "Remote port in the tunnel example")
	root.AddCommand(connect)
}
