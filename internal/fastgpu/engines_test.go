package fastgpu

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestPersistentModelPath(t *testing.T) {
	root := t.TempDir()
	outside := t.TempDir()
	inside := filepath.Join(root, "my model.gguf")
	_ = os.WriteFile(inside, []byte("GGUF"), 0600)
	if p, e := persistentPath(root, inside); e != nil || p != inside {
		t.Fatal(p, e)
	}
	link := filepath.Join(root, "escape")
	if e := os.Symlink(outside, link); e != nil {
		t.Fatal(e)
	}
	if _, e := persistentPath(root, link); e == nil {
		t.Fatal("accepted ephemeral target through symlink")
	}
	for _, bad := range []string{"/tmp/model.gguf", "/workspace/../root/model.gguf", "vol:../bad"} {
		if _, e := modelPath(bad); e == nil {
			t.Fatal("accepted", bad)
		}
	}
}

func TestGGUFFingerprintTracksChanges(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "model.gguf")
	cache := filepath.Join(dir, "checks")
	data := []byte("GGUF" + strings.Repeat("a", 64))
	_ = os.WriteFile(file, data, 0600)
	first, e := ggufFingerprint(file, cache)
	if e != nil {
		t.Fatal(e)
	}
	again, e := ggufFingerprint(file, cache)
	if e != nil || again != first {
		t.Fatal(again, e)
	}
	data[20] = 'b'
	_ = os.WriteFile(file, data, 0600)
	now := time.Now().Add(time.Second)
	_ = os.Chtimes(file, now, now)
	changed, e := ggufFingerprint(file, cache)
	if e != nil || changed == first {
		t.Fatal(changed, e)
	}
	_ = os.WriteFile(file, []byte(strings.Repeat("invalid", 10)), 0600)
	if _, e = ggufFingerprint(file, cache); e == nil {
		t.Fatal("accepted non-GGUF")
	}
}

func TestDeploymentValidation(t *testing.T) {
	base := Deployment{Model: "my-model", Engine: "llama.cpp", Target: "/workspace/models/custom.gguf", Context: 4096}
	if e := validateDeployment(base); e != nil {
		t.Fatal(e)
	}
	for _, arg := range []string{"--host=0.0.0.0", "--port", "--served-model-name=other", "--ho=0.0.0.0", "--config=other.yaml", "-m"} {
		d := base
		d.Args = []string{arg}
		if e := validateDeployment(d); e == nil {
			t.Fatal("accepted reserved flag", arg)
		}
	}
	d := base
	d.Args = []string{"--threads", "4", "--n-gpu-layers", "-1"}
	if e := validateDeployment(d); e != nil {
		t.Fatal(e)
	}
	d = base
	d.Engine = "vllm"
	if e := validateDeployment(d); e == nil {
		t.Fatal("GGUF without tokenizer accepted")
	}
	d.Tokenizer = "org/tokenizer"
	if e := validateDeployment(d); e != nil {
		t.Fatal(e)
	}
	d.HF = true
	d.Target = "../.."
	if e := validateDeployment(d); e == nil {
		t.Fatal("local traversal as HF ID")
	}
	d.Target = "org/model"
	if e := validateDeployment(d); e != nil {
		t.Fatal(e)
	}
	d = base
	d.Engine = "ollama"
	d.Args = []string{"--threads", "4"}
	if e := validateDeployment(d); e == nil {
		t.Fatal("Ollama server args accepted")
	}
}

func TestEngineCommandsKeepModelAndLoopback(t *testing.T) {
	for _, engine := range []string{"llama.cpp", "ollama", "vllm"} {
		d := Deployment{Model: "custom", Engine: engine, Target: "/workspace/file with spaces.gguf", Context: 2048, Tokenizer: "org/base"}
		bin, args := engineCommand(d)
		if engine == "ollama" {
			if bin != "ollama" || strings.Join(args, " ") != "serve" {
				t.Fatal(bin, args)
			}
			continue
		}
		hasTarget, hasLoopback := false, false
		for _, v := range args {
			hasTarget = hasTarget || v == d.Target
			hasLoopback = hasLoopback || v == "127.0.0.1"
		}
		if !hasTarget || !hasLoopback {
			t.Fatal(bin, args)
		}
	}
}

func TestReadyModelRequiresExpectedAlias(t *testing.T) {
	alias := "wrong"
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/health" {
			fmt.Fprint(w, `{"status":"ok"}`)
			return
		}
		if r.URL.Path == "/api/ps" {
			fmt.Fprintf(w, `{"models":[{"model":%q}]}`, alias)
			return
		}
		fmt.Fprintf(w, `{"data":[{"id":%q}]}`, alias)
	}))
	defer s.Close()
	d := Deployment{Model: "custom", Engine: "llama.cpp"}
	if readyModel(context.Background(), s.URL, d) == nil {
		t.Fatal("accepted another model")
	}
	alias = "custom"
	if e := readyModel(context.Background(), s.URL, d); e != nil {
		t.Fatal(e)
	}
	alias = "custom:latest"
	d.Engine = "ollama"
	if e := readyModel(context.Background(), s.URL, d); e != nil {
		t.Fatal(e)
	}
}

func TestInfoOmitsCredentialContainers(t *testing.T) {
	raw := map[string]any{"id": "pod", "gpu": map[string]any{"memory": 64}, "env": map[string]string{"TOKEN": "secret"}, "args": "--token secret", "registry": map[string]string{"password": "secret"}}
	b, e := json.Marshal(publicPodDetails(raw))
	if e != nil {
		t.Fatal(e)
	}
	if strings.Contains(string(b), "secret") || !strings.Contains(string(b), "memory") {
		t.Fatal(string(b))
	}
}

func TestConnectionsUseDedicatedSSHIdentity(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "profile with spaces")
	a := &App{Dir: dir, Config: Config{SSHKey: filepath.Join(dir, "key")}}
	p := Pod{ID: "test", Status: "RUNNING"}
	p.SSH.Direct = &Endpoint{Host: "203.0.113.10", Port: 22022, Username: "root"}
	c, e := a.connections(p, 8081, 8080)
	if e != nil {
		t.Fatal(e)
	}
	if c["tcp"] != "203.0.113.10:22022" || !strings.Contains(c["ssh"], "gpu@203.0.113.10") || !strings.Contains(c["tunnel_tcp"], "127.0.0.1:8081:127.0.0.1:8080") {
		t.Fatal(c)
	}
	b, e := os.ReadFile(filepath.Join(dir, "ssh_config"))
	if e != nil || !strings.Contains(string(b), "User gpu") {
		t.Fatal(string(b), e)
	}
	p.Status = "EXITED"
	c, e = a.connections(p, 8081, 8080)
	if e != nil || c["ssh"] != "" {
		t.Fatal(c, e)
	}
}

func TestCapacityRejectionDoesNotTreatAmbiguousErrorsAsRetryable(t *testing.T) {
	for _, detail := range []string{"insufficient capacity", "There are no longer any instances available with the requested specifications. Please refresh and try again."} {
		if !capacityRejected(&APIError{Status: 400, Detail: detail}) {
			t.Fatal("capacity rejection not recognized", detail)
		}
	}
	for _, e := range []error{&APIError{Status: 503, Detail: "capacity unknown"}, &APIError{Status: 400, Detail: "invalid image"}, fmt.Errorf("timeout")} {
		if capacityRejected(e) {
			t.Fatal("unsafe retry", e)
		}
	}
}
