package fastgpu

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

const ModelAlias = "qwen3.8-27b"
const modelFile = "Qwen3.8-27B-UD-Q4_K_M.gguf"
const modelSHA = "322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482"
const modelSize int64 = 16464440224
const modelURL = "https://huggingface.co/unsloth/Qwen3.8-27B-GGUF/resolve/4ca720788d1e01f1bff70c033e0d0028fd02e502/" + modelFile
const modelDir = "/workspace/.gpu/models"
const deployFile = "/workspace/.gpu/deployment.json"

type Deployment struct {
	Model     string   `json:"model"`
	Engine    string   `json:"engine"`
	Target    string   `json:"target"`
	HF        bool     `json:"hf,omitempty"`
	Tokenizer string   `json:"tokenizer,omitempty"`
	Args      []string `json:"args,omitempty"`
	Modelfile string   `json:"modelfile,omitempty"`
	Context   int      `json:"context"`
	SHA256    string   `json:"sha256"`
}
type Verification struct {
	Size     int64  `json:"size"`
	Modified int64  `json:"modified"`
	SHA256   string `json:"sha256"`
}

func verifyModel(file string) error {
	st, err := os.Stat(file)
	if err != nil {
		return err
	}
	if st.Size() != modelSize {
		return fmt.Errorf("incorrect GGUF size: %d, expected %d", st.Size(), modelSize)
	}
	var cached Verification
	b, _ := os.ReadFile(file + ".verified.json")
	_ = json.Unmarshal(b, &cached)
	if cached.Size == st.Size() && cached.Modified == st.ModTime().UnixNano() && cached.SHA256 == modelSHA {
		return nil
	}
	f, err := os.Open(file)
	if err != nil {
		return err
	}
	defer f.Close()
	h := sha256.New()
	if _, err = io.Copy(h, f); err != nil {
		return err
	}
	if hex.EncodeToString(h.Sum(nil)) != modelSHA {
		return fmt.Errorf("invalid GGUF checksum: %s", file)
	}
	return atomicJSON(file+".verified.json", Verification{st.Size(), st.ModTime().UnixNano(), modelSHA})
}

func prepareModel(ctx context.Context) error {
	if _, err := os.Stat("/run/gpu/ready"); err != nil {
		return fmt.Errorf("fastgpu environment is not initialized")
	}
	if err := os.MkdirAll(modelDir, 0755); err != nil {
		return err
	}
	l, err := os.OpenFile(filepath.Join(modelDir, "download.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer l.Close()
	if err = syscall.Flock(int(l.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		return fmt.Errorf("a model download is already in progress")
	}
	defer syscall.Flock(int(l.Fd()), syscall.LOCK_UN)
	dest := filepath.Join(modelDir, modelFile)
	if _, err = os.Stat(dest); err == nil {
		if err = verifyModel(dest); err != nil {
			return err
		}
		fmt.Println("GGUF already present and verified")
		return nil
	} else if !os.IsNotExist(err) {
		return err
	}
	f, err := os.OpenFile(dest+".partial", os.O_CREATE|os.O_RDWR, 0644)
	if err != nil {
		return err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return err
	}
	offset := st.Size()
	if offset > modelSize {
		return fmt.Errorf("partial download is larger than the model: move it before retrying")
	}
	var stat syscall.Statfs_t
	if err = syscall.Statfs(modelDir, &stat); err != nil {
		return err
	}
	if int64(stat.Bavail)*int64(stat.Bsize) < modelSize-offset+(1<<30) {
		return fmt.Errorf("insufficient volume space: at least %.1f GB of free space required", float64(modelSize-offset+(1<<30))/1e9)
	}
	if offset < modelSize {
		req, err := http.NewRequestWithContext(ctx, "GET", modelURL, nil)
		if err != nil {
			return err
		}
		req.Header.Set("Accept-Encoding", "identity")
		if offset > 0 {
			req.Header.Set("Range", fmt.Sprintf("bytes=%d-", offset))
		}
		httpClient := &http.Client{Timeout: 3 * time.Hour}
		resp, err := httpClient.Do(req)
		if err != nil {
			return err
		}
		defer resp.Body.Close()
		if resp.StatusCode != 200 && resp.StatusCode != 206 {
			return fmt.Errorf("GGUF download: HTTP %d", resp.StatusCode)
		}
		if resp.StatusCode == 206 && !strings.HasPrefix(resp.Header.Get("Content-Range"), fmt.Sprintf("bytes %d-", offset)) {
			return fmt.Errorf("invalid download Range response")
		}
		if resp.StatusCode == 200 {
			offset = 0
			if err = f.Truncate(0); err != nil {
				return err
			}
		}
		if _, err = f.Seek(offset, io.SeekStart); err != nil {
			return err
		}
		buf := make([]byte, 256*1024)
		last := time.Now()
		read := io.LimitReader(resp.Body, modelSize-offset+1)
		for {
			n, e := read.Read(buf)
			if n > 0 {
				if _, err = f.Write(buf[:n]); err != nil {
					return err
				}
				offset += int64(n)
			}
			if time.Since(last) > 5*time.Second {
				fmt.Fprintf(os.Stderr, "GGUF: %.1f / %.1f GB\n", float64(offset)/1e9, float64(modelSize)/1e9)
				last = time.Now()
			}
			if e == io.EOF {
				break
			}
			if e != nil {
				return e
			}
		}
	}
	if err = f.Sync(); err != nil {
		return err
	}
	if err = f.Close(); err != nil {
		return err
	}
	if err = verifyModel(dest + ".partial"); err != nil {
		return err
	}
	if err = os.Rename(dest+".partial", dest); err != nil {
		return err
	}
	_ = os.Rename(dest+".partial.verified.json", dest+".verified.json")
	fmt.Println("GGUF downloaded and SHA-256 verified")
	return nil
}
func supervisor(ctx context.Context, action string) error {
	cmd := exec.CommandContext(ctx, "supervisorctl", "-c", "/etc/supervisor/supervisord.conf", action, "inference")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
func health(ctx context.Context, base string) error {
	r, e := http.NewRequestWithContext(ctx, "GET", base+"/health", nil)
	if e != nil {
		return e
	}
	c := &http.Client{Timeout: 5 * time.Second}
	resp, e := c.Do(r)
	if e != nil {
		return e
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("server not ready: HTTP %d", resp.StatusCode)
	}
	return nil
}
func workerServe() error {
	b, err := os.ReadFile(deployFile)
	if err != nil {
		return err
	}
	var d Deployment
	if err = json.Unmarshal(b, &d); err != nil {
		return err
	}
	if err = validateDeployment(d); err != nil {
		return err
	}
	sum, err := deploymentFingerprint(d)
	if err != nil {
		return err
	}
	if sum != d.SHA256 {
		return fmt.Errorf("model changed: run fastgpu deploy again before restarting the service")
	}
	name, args := engineCommand(d)
	bin, err := exec.LookPath(name)
	if err != nil {
		return err
	}
	return syscall.Exec(bin, append([]string{bin}, args...), engineEnv())
}
func workerCheck(ctx context.Context) error {
	checks := [][]string{{"python", "--version"}, {"uv", "--version"}, {"gh", "--version"}, {"codex", "--version"}, {"llama-server", "--version"}, {"ollama", "--version"}, {"nvidia-smi", "--query-gpu=name,memory.total,driver_version", "--format=csv,noheader"}, {"python", "-c", `import torch, vllm; print('vLLM', vllm.__version__, 'PyTorch', torch.__version__, 'CUDA', torch.version.cuda); assert torch.cuda.is_available(), 'CUDA is not available'; x=torch.ones((32,32),device='cuda'); assert (x@x)[0,0].item()==32; print('CUDA computation succeeded')`}}
	for _, args := range checks {
		fmt.Println("Checking:", args[0])
		cmd := exec.CommandContext(ctx, args[0], args[1:]...)
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			return fmt.Errorf("%s: %w", args[0], err)
		}
	}
	return nil
}
