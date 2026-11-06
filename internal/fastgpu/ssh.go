package fastgpu

import (
	"context"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"golang.org/x/term"
)

func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", "'\"'\"'") + "'" }
func shellWords(args []string) string {
	q := make([]string, len(args))
	for i, s := range args {
		q[i] = quote(s)
	}
	return strings.Join(q, " ")
}
func remotePath(s string) (string, error) {
	if strings.ContainsAny(s, "\x00\r\n") {
		return "", fmt.Errorf("invalid path")
	}
	base := remoteHome
	if strings.HasPrefix(s, "vol:") {
		base = workspace
		s = strings.TrimPrefix(s, "vol:")
	} else {
		s = strings.TrimPrefix(s, "home:")
	}
	if strings.HasPrefix(s, "/") {
		return "", fmt.Errorf("use a relative path with home: or vol:")
	}
	p := path.Join(base, s)
	if p != base && !strings.HasPrefix(p, base+"/") {
		return "", fmt.Errorf("path escapes the selected root")
	}
	return p, nil
}

var sshSafe = regexp.MustCompile(`^[a-zA-Z0-9._:-]+$`)

func (a *App) sshConfig(p Pod) error {
	if p.SSH.Direct == nil {
		return fmt.Errorf("Pod %s: direct SSH is not available yet", p.ID)
	}
	e := p.SSH.Direct
	if !sshSafe.MatchString(e.Host) || !sshSafe.MatchString(p.ID) || e.Port < 1 || e.Port > 65535 {
		return fmt.Errorf("invalid SSH endpoint")
	}
	if strings.ContainsAny(a.Config.SSHKey+a.Dir, "\r\n\x00") {
		return fmt.Errorf("invalid SSH configuration path")
	}
	if err := os.MkdirAll(a.Dir, 0700); err != nil {
		return err
	}
	content := fmt.Sprintf("Host gpu-%s\n  HostName %s\n  Port %d\n  User gpu\n  IdentityFile %q\n  IdentitiesOnly yes\n  UserKnownHostsFile %q\n  HostKeyAlias gpu-%s\n  StrictHostKeyChecking accept-new\n  ConnectTimeout 10\n  ServerAliveInterval 20\n  ServerAliveCountMax 3\n  ForwardAgent no\n", p.ID, e.Host, e.Port, a.Config.SSHKey, filepath.Join(a.Dir, "known_hosts"), p.ID)
	return atomicFile(filepath.Join(a.Dir, "ssh_config"), []byte(content))
}
func (a *App) sshArgs(p Pod, tty bool) []string {
	args := []string{"-F", filepath.Join(a.Dir, "ssh_config"), "-o", "BatchMode=yes"}
	if tty {
		args = append(args, "-tt")
	}
	return append(args, "gpu-"+p.ID)
}
func (a *App) remote(ctx context.Context, p Pod, args []string, stdin io.Reader, out, stderr io.Writer, tty bool) error {
	if err := a.sshConfig(p); err != nil {
		return err
	}
	argv := a.sshArgs(p, tty)
	if len(args) > 0 {
		argv = append(argv, "exec bash -lc "+quote("exec "+shellWords(args)))
	}
	cmd := exec.CommandContext(ctx, "ssh", argv...)
	cmd.Stdin = stdin
	cmd.Stdout = out
	cmd.Stderr = stderr
	if tty && term.IsTerminal(int(os.Stdin.Fd())) {
		// OpenSSH reads the original terminal settings first and handles raw mode itself.
		old, err := term.GetState(int(os.Stdin.Fd()))
		if err != nil {
			return err
		}
		defer term.Restore(int(os.Stdin.Fd()), old)
	}
	return cmd.Run()
}
func (a *App) remoteOutput(ctx context.Context, p Pod, args ...string) (string, error) {
	var b strings.Builder
	err := a.remote(ctx, p, args, nil, &b, os.Stderr, false)
	return b.String(), err
}
func (a *App) selected(ctx context.Context, c *API) (Pod, error) {
	if a.State.PodID == "" {
		return Pod{}, fmt.Errorf("no Pod selected: use fastgpu up")
	}
	return c.pod(ctx, a.State.PodID)
}
func (a *App) ensureKey(ctx context.Context) error {
	if _, err := os.Stat(a.Config.SSHKey); err == nil {
		_, err = os.Stat(a.Config.SSHKey + ".pub")
		return err
	} else if !os.IsNotExist(err) {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(a.Config.SSHKey), 0700); err != nil {
		return err
	}
	cmd := exec.CommandContext(ctx, "ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fastgpu-workspace", "-f", a.Config.SSHKey)
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
func (a *App) socket(p Pod) string { return filepath.Join(a.Dir, "tunnel-"+p.ID) }
func (a *App) tunnel(ctx context.Context, p Pod, port int) error {
	if port < 1 || port > 65535 {
		return fmt.Errorf("invalid local port")
	}
	if err := a.sshConfig(p); err != nil {
		return err
	}
	base := []string{"-F", filepath.Join(a.Dir, "ssh_config"), "-S", a.socket(p)}
	check := exec.CommandContext(ctx, "ssh", append(append([]string{}, base...), "-O", "check", "gpu-"+p.ID)...)
	if check.Run() == nil {
		if a.State.TunnelPort == port {
			return nil
		}
		return fmt.Errorf("tunnel already exists on port %d: close it with fastgpu tunnel --close", a.State.TunnelPort)
	}
	l, err := net.Listen("tcp", net.JoinHostPort("127.0.0.1", strconv.Itoa(port)))
	if err != nil {
		return fmt.Errorf("port %d is in use: choose --local-port", port)
	}
	l.Close()
	args := append(base, "-M", "-fN", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes", "-L", fmt.Sprintf("127.0.0.1:%d:127.0.0.1:8080", port), "gpu-"+p.ID)
	cmd := exec.CommandContext(ctx, "ssh", args...)
	cmd.Stderr = os.Stderr
	if err = cmd.Run(); err != nil {
		return err
	}
	a.State.TunnelPort = port
	return a.saveState()
}
func (a *App) closeTunnel(ctx context.Context, p Pod) {
	if err := a.sshConfig(p); err != nil {
		return
	}
	cmd := exec.CommandContext(ctx, "ssh", "-F", filepath.Join(a.Dir, "ssh_config"), "-S", a.socket(p), "-O", "exit", "gpu-"+p.ID)
	_ = cmd.Run()
	a.State.TunnelPort = 0
}
func (a *App) transfer(ctx context.Context, p Pod, local, remote string, push bool) error {
	r, err := remotePath(remote)
	if err != nil {
		return err
	}
	if err = a.sshConfig(p); err != nil {
		return err
	}
	localContents := strings.HasSuffix(local, "/")
	remoteContents := strings.HasSuffix(remote, "/")
	local, err = filepath.Abs(local)
	if err != nil {
		return err
	}
	if localContents {
		local += "/"
	}
	if remoteContents {
		r += "/"
	}
	args := []string{"-a", "--no-owner", "--no-group", "--protect-args", "--partial", "--partial-dir=.gpu-rsync-partial", "--info=progress2", "-e", shellWords([]string{"ssh", "-F", filepath.Join(a.Dir, "ssh_config"), "-o", "BatchMode=yes"})}
	for _, s := range []string{".venv/", "__pycache__/", ".env", ".env.*", "*.pem", "*.key"} {
		args = append(args, "--exclude", s)
	}
	if push {
		if err = a.remote(ctx, p, []string{"mkdir", "-p", "--", r}, nil, os.Stdout, os.Stderr, false); err != nil {
			return err
		}
		args = append(args, "--", local, "gpu-"+p.ID+":"+r+"/")
	} else {
		if err = os.MkdirAll(local, 0755); err != nil {
			return err
		}
		args = append(args, "--", "gpu-"+p.ID+":"+r, local+"/")
	}
	cmd := exec.CommandContext(ctx, "rsync", args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
func (a *App) openCode(ctx context.Context, p Pod, folder string) error {
	if err := a.sshConfig(p); err != nil {
		return err
	}
	r, err := remotePath(folder)
	if err != nil {
		return err
	}
	if _, err = a.remoteOutput(ctx, p, "test", "-d", r); err != nil {
		return fmt.Errorf("remote directory not found: %s", r)
	}
	h, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	sshDir := filepath.Join(h, ".ssh")
	if err = os.MkdirAll(sshDir, 0700); err != nil {
		return err
	}
	configFile := filepath.Join(sshDir, "config")
	b, err := os.ReadFile(configFile)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	include := fmt.Sprintf("Include %q", filepath.Join(a.Dir, "ssh_config"))
	if !strings.Contains(string(b), include) {
		f, err := os.OpenFile(configFile, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0600)
		if err != nil {
			return err
		}
		_, err = f.Write(append([]byte(include+"\n"), b...))
		closeErr := f.Close()
		if err != nil {
			return err
		}
		if closeErr != nil {
			return closeErr
		}
	}
	cmd := exec.CommandContext(ctx, "code", "--new-window", "--remote", "ssh-remote+gpu-"+p.ID, r)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
