package fastgpu

import (
	"context"
	"crypto/sha256"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestPresetDownloads(t *testing.T) {
	for alias, preset := range modelPresets {
		for _, mode := range []string{"resume", "restart", "bad checksum"} {
			t.Run(alias+"/"+mode, func(t *testing.T) {
				data := "GGUF" + strings.Repeat(alias, 8)
				requests := 0
				s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					requests++
					if r.URL.Path != "/"+preset.File || r.Header.Get("Range") != "bytes=8-" {
						t.Error("incorrect download target or resume offset", r.URL.Path, r.Header)
					}
					if mode == "restart" {
						fmt.Fprint(w, data)
						return
					}
					w.Header().Set("Content-Range", fmt.Sprintf("bytes 8-%d/%d", len(data)-1, len(data)))
					w.WriteHeader(http.StatusPartialContent)
					fmt.Fprint(w, data[8:])
				}))
				defer s.Close()
				model := preset
				model.URL = s.URL + "/" + model.File
				model.Size = int64(len(data))
				model.SHA256 = fmt.Sprintf("%x", sha256.Sum256([]byte(data)))
				if mode == "bad checksum" {
					model.SHA256 = strings.Repeat("0", 64)
				}
				dir := t.TempDir()
				file := filepath.Join(dir, model.File)
				if err := os.WriteFile(file+".partial", []byte(data[:8]), 0600); err != nil {
					t.Fatal(err)
				}
				err := downloadModel(context.Background(), dir, model)
				if mode == "bad checksum" {
					if err == nil || !strings.Contains(err.Error(), "checksum") {
						t.Fatal("corrupted download accepted", err)
					}
					if _, err = os.Stat(file); !os.IsNotExist(err) {
						t.Fatal("unverified model was published", err)
					}
					return
				}
				if err != nil {
					t.Fatal(err)
				}
				got, err := os.ReadFile(file)
				if err != nil || string(got) != data {
					t.Fatal("download contents differ", err)
				}
				if _, err = os.Stat(file + ".partial"); !os.IsNotExist(err) {
					t.Fatal("partial file not renamed", err)
				}
				if err = downloadModel(context.Background(), dir, model); err != nil || requests != 1 {
					t.Fatal("verified model was not reused", requests, err)
				}
				model.SHA256 = strings.Repeat("0", 64)
				if err = verifyModel(file, model); err == nil {
					t.Fatal("verification cache accepted a different expected checksum")
				}
			})
		}
	}
}

func TestModelListDeployTargets(t *testing.T) {
	root := t.TempDir()
	managed := filepath.Join(root, ".gpu", "models")
	custom := filepath.Join(root, "models", "my model's weights.gguf")
	files := []string{custom, filepath.Join(managed, "incomplete.gguf.partial"), filepath.Join(managed, "cached.gguf.verified.json")}
	for _, preset := range modelPresets {
		files = append(files, filepath.Join(managed, preset.File))
	}
	// A preset filename outside the managed directory must be deployed by path.
	copy := filepath.Join(root, "models", modelPresets[ModelAlias].File)
	files = append(files, copy)
	for _, file := range files {
		if err := os.MkdirAll(filepath.Dir(file), 0755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(file, []byte("GGUF"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	outside := t.TempDir()
	if err := os.Symlink(outside, filepath.Join(root, "external")); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(files[0], filepath.Join(managed, "linked.gguf")); err != nil {
		t.Fatal(err)
	}
	var out strings.Builder
	if err := listModels(&out, root, root); err != nil {
		t.Fatal(err)
	}
	for _, target := range append(presetAliases(), quote(custom), quote(copy)) {
		if !strings.Contains(out.String(), target) {
			t.Fatal("missing deploy target", target, out.String())
		}
	}
	for _, hidden := range []string{".partial", ".verified.json", "linked.gguf"} {
		if strings.Contains(out.String(), hidden) {
			t.Fatal("listed an incomplete file, cache, or symlink", out.String())
		}
	}
	out.Reset()
	if err := listModels(&out, root, managed); err != nil || strings.Contains(out.String(), quote(custom)) {
		t.Fatal("directory selection ignored", out.String(), err)
	}
	if err := listModels(&out, root, filepath.Join(root, "external")); err == nil {
		t.Fatal("listed a directory outside the volume")
	}
	empty := t.TempDir()
	out.Reset()
	if err := listModels(&out, empty, empty); err != nil || !strings.Contains(out.String(), "No GGUF models found") {
		t.Fatal(out.String(), err)
	}
}

func TestPresetValidation(t *testing.T) {
	for _, alias := range presetAliases() {
		if err := validModel(nil, []string{alias}); err != nil {
			t.Fatal(err)
		}
		d := Deployment{Model: alias, Target: alias, Engine: "llama.cpp", Context: 131072}
		if err := validateDeployment(d); err != nil {
			t.Fatal(err)
		}
		d.Engine = "vllm"
		if err := validateDeployment(d); err == nil {
			t.Fatal("preset accepted by vLLM without a tokenizer", alias)
		}
	}
	if err := validModel(nil, nil); err != nil {
		t.Fatal("default preset rejected", err)
	}
	if err := prepareModel(context.Background(), "unknown-model"); err == nil || !strings.Contains(err.Error(), "supported models") {
		t.Fatal("unknown preset not rejected", err)
	}
}
