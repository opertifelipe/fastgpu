package fastgpu

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func testStartupApp(t *testing.T) *App {
	t.Helper()
	t.Setenv("GPU_CONFIG_DIR", t.TempDir())
	a, err := newApp()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.release)
	return a
}

func runTestCLI(t *testing.T, a *App, args ...string) (string, string, error) {
	t.Helper()
	stdout, err := os.CreateTemp(t.TempDir(), "stdout")
	if err != nil {
		t.Fatal(err)
	}
	defer stdout.Close()
	stderr, err := os.CreateTemp(t.TempDir(), "stderr")
	if err != nil {
		t.Fatal(err)
	}
	defer stderr.Close()
	previousOut, previousErr := os.Stdout, os.Stderr
	os.Stdout, os.Stderr = stdout, stderr
	defer func() { os.Stdout, os.Stderr = previousOut, previousErr }()
	cmd := a.command(context.Background())
	cmd.SetArgs(append([]string{}, args...))
	err = cmd.Execute()
	out, readErr := os.ReadFile(stdout.Name())
	if readErr != nil {
		t.Fatal(readErr)
	}
	messages, readErr := os.ReadFile(stderr.Name())
	if readErr != nil {
		t.Fatal(readErr)
	}
	return string(out), string(messages), err
}

func TestStartupSizes(t *testing.T) {
	for _, tc := range []struct {
		name, size string
		flags      []string
		prices     []float64
	}{
		{"default", "small", nil, []float64{.4, 1.49}},
		{"small", "small", []string{"--small"}, []float64{.4, 1.49}},
		{"medium", "medium", []string{"--medium"}, []float64{3, 2.75, 3.25, 3.49, 1.5}},
		{"large", "large", []string{"--large"}, []float64{3.5, 4, 5}},
		{"xlarge", "xlarge", []string{"--xlarge"}, []float64{5.01, 6}},
		{"capped medium", "medium", []string{"--medium", "--max-price", "2.8"}, []float64{2.75, 1.5}},
		{"empty tier", "xlarge", []string{"--xlarge", "--max-price", "5"}, nil},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a := testStartupApp(t)
			a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Method != "GET" {
					t.Error("dry-run attempted a write", r.Method, r.URL.Path)
					http.Error(w, "unexpected write", 400)
					return
				}
				switch r.URL.Path {
				case "/pods":
					fmt.Fprint(w, `{"pods":[{"id":"old","name":"gpu-old","status":"TERMINATED"},{"id":"other","name":"unrelated","status":"RUNNING"}]}`)
				case "/network-volumes":
					fmt.Fprint(w, `{"networkVolumes":[{"id":"v","dataCenter":"TEST-DC"}]}`)
				case "/catalog/gpus":
					var offers []map[string]any
					for _, price := range []float64{6, 3.25, .4, 3, 5, 1.49, 4, 5.01, 3.5, 2.75, 1.5, 3.49} {
						offers = append(offers, map[string]any{
							"id": fmt.Sprint(price), "manufacturer": "NVIDIA", "memory": 24, "secure": true,
							"price":       map[string]float64{"secure": price},
							"dataCenters": []map[string]string{{"id": "TEST-DC", "availability": "LOW"}},
						})
					}
					_ = json.NewEncoder(w).Encode(map[string]any{"gpus": offers})
				default:
					t.Error("unexpected request", r.URL.Path)
					http.NotFound(w, r)
				}
			})
			out, _, err := runTestCLI(t, a, append([]string{"up", "--dry-run"}, tc.flags...)...)
			if tc.prices == nil {
				if err == nil || !strings.Contains(err.Error(), "no compatible xlarge") {
					t.Fatal("empty tier should fail without falling back", err)
				}
				return
			}
			if err != nil {
				t.Fatal(err)
			}
			var plan struct {
				Size   string
				Offers []GPU
			}
			if err = json.Unmarshal([]byte(out), &plan); err != nil {
				t.Fatal(err)
			}
			var prices []float64
			for _, offer := range plan.Offers {
				prices = append(prices, offer.Price.Secure)
			}
			if plan.Size != tc.size || !reflect.DeepEqual(prices, tc.prices) {
				t.Fatalf("got %s %v, want %s %v", plan.Size, prices, tc.size, tc.prices)
			}
		})
	}
}

func TestStartupRejectsConflictingSizes(t *testing.T) {
	sizes := []string{"small", "medium", "large", "xlarge"}
	for i, first := range sizes {
		for _, second := range sizes[i+1:] {
			t.Run(first+"+"+second, func(t *testing.T) {
				a := testStartupApp(t)
				a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
					t.Error("conflicting flags reached the API")
					http.Error(w, "unexpected request", 400)
				})
				if _, _, err := runTestCLI(t, a, "up", "--"+first, "--"+second); err == nil {
					t.Fatal("conflicting sizes accepted")
				}
			})
		}
	}
}

func TestStartupCreationStaysInSize(t *testing.T) {
	for _, tc := range []struct {
		name string
		args []string
		want []string
	}{
		{"bare command", nil, []string{"small"}},
		{"medium capacity retries", []string{"up", "--medium"}, []string{"medium", "medium-fallback"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a := testStartupApp(t)
			for _, suffix := range []string{"", ".pub"} {
				if err := os.WriteFile(a.Config.SSHKey+suffix, []byte("test-key"), 0600); err != nil {
					t.Fatal(err)
				}
			}
			var requested []string
			a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
				switch {
				case r.Method == "POST" && r.URL.Path == "/pods":
					var body Pod
					if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
						t.Error(err)
					}
					requested = append(requested, body.GPU.ID)
					w.WriteHeader(400)
					fmt.Fprint(w, `{"detail":"capacity exhausted"}`)
				case r.Method == "GET" && r.URL.Path == "/pods":
					fmt.Fprint(w, `{"pods":[]}`)
				case r.Method == "GET" && r.URL.Path == "/network-volumes":
					fmt.Fprint(w, `{"networkVolumes":[{"id":"v","dataCenter":"TEST-DC"}]}`)
				case r.Method == "GET" && r.URL.Path == "/catalog/gpus":
					fmt.Fprint(w, `{"gpus":[
						{"id":"small","manufacturer":"NVIDIA","memory":24,"secure":true,"price":{"secure":0.4},"dataCenters":[{"id":"TEST-DC","availability":"LOW"}]},
						{"id":"medium-fallback","manufacturer":"NVIDIA","memory":24,"secure":true,"price":{"secure":2.5},"dataCenters":[{"id":"TEST-DC","availability":"LOW"}]},
						{"id":"medium","manufacturer":"NVIDIA","memory":24,"secure":true,"price":{"secure":3},"dataCenters":[{"id":"TEST-DC","availability":"LOW"}]},
						{"id":"large","manufacturer":"NVIDIA","memory":24,"secure":true,"price":{"secure":4},"dataCenters":[{"id":"TEST-DC","availability":"LOW"}]}
					]}`)
				default:
					t.Error("unexpected request", r.Method, r.URL.Path)
					http.NotFound(w, r)
				}
			})
			_, _, err := runTestCLI(t, a, tc.args...)
			if !capacityRejected(err) || !reflect.DeepEqual(requested, tc.want) {
				t.Fatal("unexpected creation attempts", requested, err)
			}
		})
	}
}

func TestStartupBlocksUnselectedPod(t *testing.T) {
	for _, status := range []string{"RUNNING", "EXITED", "CREATED"} {
		t.Run(status, func(t *testing.T) {
			a := testStartupApp(t)
			a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Method != "GET" || r.URL.Path != "/pods" {
					t.Error("attempted startup with an existing Pod", r.Method, r.URL.Path)
					http.Error(w, "unexpected request", 400)
					return
				}
				if r.URL.Query().Get("cursor") == "next" {
					fmt.Fprintf(w, `{"pods":[{"id":"existing","name":"gpu-existing","status":%q}]}`, status)
				} else {
					fmt.Fprint(w, `{"pods":[],"pagination":{"hasNextPage":true,"nextCursor":"next"}}`)
				}
			})
			if _, _, err := runTestCLI(t, a, "up", "--large"); err == nil || !strings.Contains(err.Error(), "already exists") {
				t.Fatal("unselected Pod did not block creation", err)
			}
		})
	}
}

func TestStartupFailsWhenPodListingFails(t *testing.T) {
	a := testStartupApp(t)
	a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "GET" || r.URL.Path != "/pods" {
			t.Error("attempted startup without checking Pods", r.Method, r.URL.Path)
		}
		http.Error(w, "forbidden", 403)
	})
	if _, _, err := runTestCLI(t, a, "up"); !isStatus(err, 403) {
		t.Fatal("Pod listing failure was ignored", err)
	}
}

func TestStartupReusesSelectedPod(t *testing.T) {
	for _, status := range []string{"RUNNING", "EXITED"} {
		t.Run(status, func(t *testing.T) {
			a := testStartupApp(t)
			stubRemoteCommands(t)
			a.State = State{PodID: "existing", VolumeID: "v", Managed: true}
			current, starts := status, 0
			a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Method == "POST" && r.URL.Path == "/pods/existing/action" {
					var action map[string]string
					_ = json.NewDecoder(r.Body).Decode(&action)
					if action["action"] != "start" {
						t.Error("unexpected action", action)
					}
					starts++
					current = "RUNNING"
					w.WriteHeader(204)
					return
				}
				if r.Method != "GET" || r.URL.Path != "/pods/existing" {
					t.Error("attempted to replace the selected Pod", r.Method, r.URL.Path)
					http.Error(w, "unexpected request", 400)
					return
				}
				fmt.Fprintf(w, `{"id":"existing","status":%q,"ssh":{"direct":{"host":"203.0.113.10","port":22022}}}`, current)
			})
			for _, args := range [][]string{nil, {"up", "--xlarge", "--skip-model"}} {
				_, messages, err := runTestCLI(t, a, args...)
				if err != nil || !strings.Contains(messages, "already exists") || !strings.Contains(messages, "fastgpu down") {
					t.Fatal(messages, err)
				}
			}
			if (status == "EXITED" && starts != 1) || (status == "RUNNING" && starts != 0) {
				t.Fatal("unexpected resume count", starts)
			}
		})
	}
}

func stubRemoteCommands(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	argsFile := filepath.Join(dir, "code-args")
	t.Setenv("TEST_CODE_ARGS", argsFile)
	for name, script := range map[string]string{
		"ssh":  "#!/bin/sh\nexit 0\n",
		"code": "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TEST_CODE_ARGS\"\n",
	} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(script), 0700); err != nil {
			t.Fatal(err)
		}
	}
	return argsFile
}

func TestCodeOpensHomeOrProject(t *testing.T) {
	for _, tc := range []struct {
		folder string
		want   string
	}{
		{"", "/home/gpu"},
		{"projects/demo", "/home/gpu/projects/demo"},
		{"home:project with spaces", "/home/gpu/project with spaces"},
		{"vol:projects/demo", "/workspace/projects/demo"},
	} {
		t.Run(tc.folder, func(t *testing.T) {
			a := testStartupApp(t)
			argsFile := stubRemoteCommands(t)
			a.State.PodID = "existing"
			a.api = testAPI(t, func(w http.ResponseWriter, r *http.Request) {
				if r.Method != "GET" || r.URL.Path != "/pods/existing" {
					t.Error("unexpected request", r.Method, r.URL.Path)
				}
				fmt.Fprint(w, `{"id":"existing","status":"RUNNING","ssh":{"direct":{"host":"203.0.113.10","port":22022}}}`)
			})
			args := []string{"code"}
			if tc.folder != "" {
				args = append(args, tc.folder)
			}
			if _, _, err := runTestCLI(t, a, args...); err != nil {
				t.Fatal(err)
			}
			got, err := os.ReadFile(argsFile)
			want := "--new-window\n--remote\nssh-remote+gpu-existing\n" + tc.want + "\n"
			if err != nil || string(got) != want {
				t.Fatalf("VS Code arguments = %q, want %q: %v", got, want, err)
			}
		})
	}
}
