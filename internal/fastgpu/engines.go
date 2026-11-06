package fastgpu

import (
	"bytes"
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
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"time"
)

var modelName = regexp.MustCompile(`^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$`)

func defaultModelName(target string) string {
	name := strings.ToLower(strings.TrimSuffix(filepath.Base(target), ".gguf"))
	name = regexp.MustCompile(`[^a-zA-Z0-9._-]+`).ReplaceAllString(name, "-")
	name = strings.Trim(name, ".-_")
	if len(name) > 96 {
		name = name[:96]
	}
	if name == "" {
		return "model"
	}
	return name
}

func modelPath(s string) (string, error) {
	if filepath.IsAbs(s) {
		p := filepath.Clean(s)
		if strings.ContainsAny(p, "\x00\r\n") {
			return "", fmt.Errorf("invalid path")
		}
		if p == workspace || strings.HasPrefix(p, workspace+"/") || p == remoteHome || strings.HasPrefix(p, remoteHome+"/") {
			return p, nil
		}
		return "", fmt.Errorf("use a path on the volume: /workspace/... or /home/gpu/...")
	}
	return remotePath(s)
}

func validateDeployment(d Deployment) error {
	if d.Engine != "llama.cpp" && d.Engine != "ollama" && d.Engine != "vllm" {
		return fmt.Errorf("supported engines: llama.cpp, ollama, vllm")
	}
	if !modelName.MatchString(d.Model) {
		return fmt.Errorf("invalid model name: use letters, numbers, dots, hyphens, or underscores")
	}
	if d.Context < 512 || d.Context > 262144 {
		return fmt.Errorf("allowed context size: 512..262144 tokens")
	}
	if d.HF && (d.Engine != "vllm" || !regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*$`).MatchString(d.Target)) {
		return fmt.Errorf("--hf requires vllm and a Hugging Face organization/model ID")
	}
	if d.Target == "" || strings.ContainsAny(d.Target+d.Tokenizer+d.Modelfile, "\x00\r\n") {
		return fmt.Errorf("invalid model or tokenizer path")
	}
	if d.Modelfile != "" && d.Engine != "ollama" {
		return fmt.Errorf("--modelfile requires Ollama")
	}
	if d.Tokenizer != "" && d.Engine != "vllm" {
		return fmt.Errorf("--tokenizer requires vLLM")
	}
	if d.Engine == "vllm" && !d.HF && d.Tokenizer == "" && (modelPresets[d.Target].File != "" || strings.HasSuffix(strings.ToLower(d.Target), ".gguf")) {
		return fmt.Errorf("for GGUF with vLLM, specify --tokenizer with a Hugging Face ID or persistent path")
	}
	if d.Engine == "ollama" && len(d.Args) != 0 {
		return fmt.Errorf("use --modelfile to customize Ollama, not --arg")
	}
	// The CLI manages these parameters to keep endpoints and aliases consistent.
	reserved := []string{"--host", "--port", "--model", "--alias", "--served-model-name", "--ctx-size", "--max-model-len", "--tokenizer", "--config", "--api-key", "--ssl", "--rpc", "--download-dir", "--daemon", "--help", "--version"}
	for _, arg := range d.Args {
		if strings.ContainsRune(arg, 0) {
			return fmt.Errorf("invalid argument")
		}
		_, numberErr := strconv.ParseFloat(arg, 64)
		if strings.HasPrefix(arg, "-") && !strings.HasPrefix(arg, "--") && numberErr != nil {
			return fmt.Errorf("use long options with --arg: %s", arg)
		}
		flag := strings.SplitN(arg, "=", 2)[0]
		for _, r := range reserved {
			if strings.HasPrefix(r, flag) && strings.HasPrefix(flag, "--") || strings.HasPrefix(flag, "--ssl-") {
				return fmt.Errorf("option reserved for the CLI: %s", flag)
			}
		}
	}
	return nil
}

func persistentPath(root, name string) (string, error) {
	base, err := filepath.EvalSymlinks(root)
	if err != nil {
		return "", err
	}
	p, err := filepath.EvalSymlinks(name)
	if err != nil {
		return "", err
	}
	p, err = filepath.Abs(p)
	if err != nil {
		return "", err
	}
	if p != base && !strings.HasPrefix(p, base+string(os.PathSeparator)) {
		return "", fmt.Errorf("%s is not on the Network Volume: save it in /workspace or the persistent home directory", name)
	}
	return p, nil
}

func ggufFingerprint(file, cacheDir string) (string, error) {
	f, err := os.Open(file)
	if err != nil {
		return "", err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return "", err
	}
	var magic [4]byte
	if !st.Mode().IsRegular() || st.Size() < 24 {
		return "", fmt.Errorf("invalid GGUF file")
	}
	if _, err = io.ReadFull(f, magic[:]); err != nil || string(magic[:]) != "GGUF" {
		return "", fmt.Errorf("file does not contain a GGUF header")
	}
	key := sha256.Sum256([]byte(file))
	cache := filepath.Join(cacheDir, hex.EncodeToString(key[:])+".json")
	var known Verification
	b, _ := os.ReadFile(cache)
	_ = json.Unmarshal(b, &known)
	if known.Size == st.Size() && known.Modified == st.ModTime().UnixNano() && len(known.SHA256) == 64 {
		return known.SHA256, nil
	}
	h := sha256.New()
	h.Write(magic[:])
	if _, err = io.Copy(h, f); err != nil {
		return "", err
	}
	after, err := f.Stat()
	if err != nil {
		return "", err
	}
	if after.Size() != st.Size() || !after.ModTime().Equal(st.ModTime()) {
		return "", fmt.Errorf("model changed during verification: finish saving before deployment")
	}
	sum := hex.EncodeToString(h.Sum(nil))
	return sum, atomicJSON(cache, Verification{st.Size(), st.ModTime().UnixNano(), sum})
}

func deploymentFingerprint(d Deployment) (string, error) {
	if d.HF {
		return "hf:" + d.Target, nil
	}
	file, err := persistentPath(workspace, d.Target)
	if err != nil {
		return "", err
	}
	st, err := os.Stat(file)
	if err != nil {
		return "", err
	}
	var sum string
	if st.IsDir() {
		if d.Engine == "llama.cpp" {
			return "", fmt.Errorf("llama.cpp requires a GGUF file, not a checkpoint")
		}
		if _, err = os.Stat(filepath.Join(file, "config.json")); err != nil {
			return "", fmt.Errorf("incomplete checkpoint: missing config.json")
		}
		h := sha256.New()
		err = filepath.WalkDir(file, func(p string, entry os.DirEntry, e error) error {
			if e != nil {
				return e
			}
			if entry.IsDir() {
				return nil
			}
			actual, e := persistentPath(workspace, p)
			if e != nil {
				return e
			}
			s, e := os.Stat(actual)
			if e != nil {
				return e
			}
			fmt.Fprintf(h, "%s:%d:%d\n", p, s.Size(), s.ModTime().UnixNano())
			return nil
		})
		if err != nil {
			return "", err
		}
		sum = hex.EncodeToString(h.Sum(nil))
	} else {
		sum, err = ggufFingerprint(file, filepath.Join(modelDir, ".checks"))
		if err != nil {
			return "", err
		}
		if d.Engine == "vllm" && d.Tokenizer == "" {
			return "", fmt.Errorf("for GGUF with vLLM, specify --tokenizer with a Hugging Face ID or persistent path")
		}
	}
	if d.Modelfile != "" {
		p, e := persistentPath(workspace, d.Modelfile)
		if e != nil {
			return "", e
		}
		b, e := os.ReadFile(p)
		if e != nil {
			return "", e
		}
		h := sha256.Sum256(append([]byte(sum), b...))
		sum = hex.EncodeToString(h[:])
	}
	return sum, nil
}

func engineCommand(d Deployment) (string, []string) {
	switch d.Engine {
	case "ollama":
		return "ollama", []string{"serve"}
	case "vllm":
		args := []string{"serve", d.Target, "--host", "127.0.0.1", "--port", "8080", "--served-model-name", d.Model, "--max-model-len", strconv.Itoa(d.Context)}
		if d.Tokenizer != "" {
			args = append(args, "--tokenizer", d.Tokenizer)
		}
		return "vllm", append(args, d.Args...)
	default:
		return "llama-server", append([]string{"--model", d.Target, "--alias", d.Model, "--host", "127.0.0.1", "--port", "8080", "--ctx-size", strconv.Itoa(d.Context), "--parallel", "1", "--n-gpu-layers", "all", "--fit", "off", "--jinja", "--no-mmproj"}, d.Args...)
	}
}

func engineEnv() []string {
	env := []string{}
	for _, s := range os.Environ() {
		if !strings.HasPrefix(s, "OLLAMA_HOST=") && !strings.HasPrefix(s, "OLLAMA_MODELS=") {
			env = append(env, s)
		}
	}
	return append(env, "OLLAMA_HOST=127.0.0.1:8080", "OLLAMA_MODELS=/workspace/.gpu/ollama/models")
}

func readyModel(ctx context.Context, base string, d Deployment) error {
	if d.Engine != "ollama" {
		if err := health(ctx, base); err != nil {
			return err
		}
	}
	if err := catalogModel(ctx, base, d); err != nil {
		return err
	}
	if d.Engine != "ollama" {
		return nil
	}
	r, err := http.NewRequestWithContext(ctx, "GET", base+"/api/ps", nil)
	if err != nil {
		return err
	}
	resp, err := (&http.Client{Timeout: 5 * time.Second}).Do(r)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	var loaded struct {
		Models []struct {
			Model string `json:"model"`
			Name  string `json:"name"`
		} `json:"models"`
	}
	if resp.StatusCode != 200 {
		return fmt.Errorf("Ollama not ready: HTTP %d", resp.StatusCode)
	}
	if err = json.NewDecoder(resp.Body).Decode(&loaded); err != nil {
		return err
	}
	for _, m := range loaded.Models {
		if strings.TrimSuffix(m.Model, ":latest") == d.Model || strings.TrimSuffix(m.Name, ":latest") == d.Model {
			return nil
		}
	}
	return fmt.Errorf("Ollama model exists but is not loaded")
}

func catalogModel(ctx context.Context, base string, d Deployment) error {
	r, err := http.NewRequestWithContext(ctx, "GET", base+"/v1/models", nil)
	if err != nil {
		return err
	}
	resp, err := (&http.Client{Timeout: 5 * time.Second}).Do(r)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("engine not ready: HTTP %d", resp.StatusCode)
	}
	var models struct {
		Data []struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	if err = json.NewDecoder(resp.Body).Decode(&models); err != nil {
		return err
	}
	for _, m := range models.Data {
		if strings.TrimSuffix(m.ID, ":latest") == d.Model {
			return nil
		}
	}
	return fmt.Errorf("model %s not ready", d.Model)
}

func workerDeploy(ctx context.Context, wanted Deployment) error {
	lock := &App{Dir: filepath.Dir(deployFile)}
	if err := lock.acquire(); err != nil {
		return err
	}
	defer lock.release()
	if err := validateDeployment(wanted); err != nil {
		return err
	}
	if model, ok := modelPresets[wanted.Target]; ok {
		if err := prepareModel(ctx, wanted.Target); err != nil {
			return err
		}
		wanted.Target = filepath.Join(modelDir, model.File)
	}
	if !wanted.HF {
		p, err := persistentPath(workspace, wanted.Target)
		if err != nil {
			return err
		}
		wanted.Target = p
	}
	var err error
	wanted.SHA256, err = deploymentFingerprint(wanted)
	if err != nil {
		return err
	}
	var previous Deployment
	b, _ := os.ReadFile(deployFile)
	_ = json.Unmarshal(b, &previous)
	if reflect.DeepEqual(previous, wanted) && readyModel(ctx, "http://127.0.0.1:8080", wanted) == nil {
		fmt.Println("Model already ready")
		return nil
	}
	// Stopping an already stopped service is normal on the first deployment.
	_ = exec.CommandContext(ctx, "supervisorctl", "-c", "/etc/supervisor/supervisord.conf", "stop", "inference").Run()
	if err = atomicJSON(deployFile, wanted); err != nil {
		return err
	}
	if err = supervisor(ctx, "start"); err != nil {
		return err
	}
	if wanted.Engine != "ollama" {
		return nil
	}
	wait, cancel := context.WithTimeout(ctx, 20*time.Minute)
	defer cancel()
	for {
		r, e := http.NewRequestWithContext(wait, "GET", "http://127.0.0.1:8080/api/version", nil)
		if e != nil {
			return e
		}
		resp, e := (&http.Client{Timeout: 3 * time.Second}).Do(r)
		if e == nil {
			resp.Body.Close()
			if resp.StatusCode == 200 {
				break
			}
		}
		if e = sleep(wait, time.Second); e != nil {
			return e
		}
	}
	modelfile := wanted.Modelfile
	if modelfile == "" {
		modelfile = "/workspace/.gpu/ollama/Modelfile"
		if err = os.MkdirAll(filepath.Dir(modelfile), 0755); err != nil {
			return err
		}
		text := "FROM " + strconv.Quote(wanted.Target) + "\nPARAMETER num_ctx " + strconv.Itoa(wanted.Context) + "\n"
		if err = os.WriteFile(modelfile, []byte(text), 0600); err != nil {
			return err
		}
	}
	if !reflect.DeepEqual(previous, wanted) || catalogModel(wait, "http://127.0.0.1:8080", wanted) != nil {
		cmd := exec.CommandContext(wait, "ollama", "create", wanted.Model, "-f", modelfile)
		cmd.Env, cmd.Stdout, cmd.Stderr = engineEnv(), os.Stdout, os.Stderr
		if err = cmd.Run(); err != nil {
			return err
		}
	}
	// Load the model now: catalog presence alone does not verify inference.
	data, _ := json.Marshal(map[string]any{"model": wanted.Model, "prompt": "", "stream": false, "keep_alive": -1})
	r, err := http.NewRequestWithContext(wait, "POST", "http://127.0.0.1:8080/api/generate", bytes.NewReader(data))
	if err != nil {
		return err
	}
	r.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(r)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("Ollama loading failed: HTTP %d; check fastgpu logs", resp.StatusCode)
	}
	_, err = io.Copy(io.Discard, resp.Body)
	return err
}

func workerReady(ctx context.Context) error {
	b, err := os.ReadFile(deployFile)
	if err != nil {
		return err
	}
	var d Deployment
	if err = json.Unmarshal(b, &d); err != nil {
		return err
	}
	return readyModel(ctx, "http://127.0.0.1:8080", d)
}
