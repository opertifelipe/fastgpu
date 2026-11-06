package fastgpu

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"

	"github.com/pelletier/go-toml/v2"
)

const Version = "0.2.1"
const remoteHome = "/home/gpu"
const workspace = "/workspace"

type Config struct {
	Image     string  `json:"image"`
	Registry  string  `json:"registry,omitempty"`
	Volume    string  `json:"volume,omitempty"`
	SSHKey    string  `json:"ssh_key"`
	MinVRAM   int     `json:"min_vram_gb"`
	MinRAM    int     `json:"min_ram_gb"`
	MinCUDA   string  `json:"min_cuda"`
	Disk      int     `json:"container_disk_gb"`
	MaxPrice  float64 `json:"max_price,omitempty"`
	Context   int     `json:"context_tokens"`
	LocalPort int     `json:"local_port"`
}

type State struct {
	PodID       string `json:"pod_id,omitempty"`
	PendingName string `json:"pending_name,omitempty"`
	VolumeID    string `json:"volume_id,omitempty"`
	Managed     bool   `json:"managed"`
	TunnelPort  int    `json:"tunnel_port,omitempty"`
}

type App struct {
	Config Config
	State  State
	Dir    string
	Input  *os.File
	lock   *os.File
	api    *API
}

func (a *App) client() (*API, error) {
	if a.api != nil {
		return a.api, nil
	}
	return client()
}

func newApp() (*App, error) {
	dir := os.Getenv("GPU_CONFIG_DIR")
	if dir == "" {
		d, err := os.UserConfigDir()
		if err != nil {
			return nil, err
		}
		dir = filepath.Join(d, "gpu")
	}
	dir, err := filepath.Abs(dir)
	if err != nil {
		return nil, err
	}
	a := &App{Dir: dir, Input: os.Stdin, Config: Config{Image: "felipeoperti/fastgpu-workspace:0.2.1", MinVRAM: 24, MinRAM: 32, MinCUDA: "13.0", Disk: 40, Context: 16384, LocalPort: 8080, SSHKey: filepath.Join(dir, "id_ed25519")}}
	for _, f := range []struct {
		name string
		dst  any
	}{{"config.json", &a.Config}, {"state.json", &a.State}} {
		b, err := os.ReadFile(filepath.Join(dir, f.name))
		if errors.Is(err, os.ErrNotExist) {
			continue
		}
		if err != nil {
			return nil, err
		}
		if err = json.Unmarshal(b, f.dst); err != nil {
			return nil, fmt.Errorf("%s: %w", f.name, err)
		}
	}
	return a, nil
}

func atomicJSON(path string, value any) error {
	b, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		return err
	}
	return atomicFile(path, append(b, '\n'))
}

func atomicFile(path string, b []byte) error {
	var err error
	if err = os.MkdirAll(filepath.Dir(path), 0700); err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(path), ".gpu-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err = f.Write(b); err != nil {
		f.Close()
		return err
	}
	if err = f.Sync(); err != nil {
		f.Close()
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(f.Name(), path)
}
func (a *App) saveConfig() error { return atomicJSON(filepath.Join(a.Dir, "config.json"), a.Config) }
func (a *App) saveState() error  { return atomicJSON(filepath.Join(a.Dir, "state.json"), a.State) }
func (a *App) acquire() error {
	if a.lock != nil {
		return nil
	}
	if err := os.MkdirAll(a.Dir, 0700); err != nil {
		return err
	}
	f, err := os.OpenFile(filepath.Join(a.Dir, "lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	if err = syscall.Flock(int(f.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		f.Close()
		return fmt.Errorf("another fastgpu operation is modifying this profile")
	}
	a.lock = f
	return nil
}
func (a *App) release() {
	if a.lock != nil {
		_ = syscall.Flock(int(a.lock.Fd()), syscall.LOCK_UN)
		_ = a.lock.Close()
		a.lock = nil
	}
}
func randomName() string {
	var b [6]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	return "gpu-" + hex.EncodeToString(b[:])
}

func apiKey() (string, error) {
	if k := strings.TrimSpace(os.Getenv("RUNPOD_API_KEY")); k != "" {
		return k, nil
	}
	h, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	b, err := os.ReadFile(filepath.Join(h, ".runpod", "config.toml"))
	if err != nil {
		return "", fmt.Errorf("set RUNPOD_API_KEY or configure runpodctl")
	}
	var c struct {
		APIKey string `toml:"apikey"`
	}
	if err = toml.Unmarshal(b, &c); err != nil {
		return "", fmt.Errorf("invalid runpodctl configuration")
	}
	if c.APIKey == "" {
		return "", fmt.Errorf("missing RunPod key")
	}
	return c.APIKey, nil
}
