package fastgpu

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func testAPI(t *testing.T, h http.HandlerFunc) *API {
	t.Helper()
	s := httptest.NewServer(h)
	t.Cleanup(s.Close)
	return &API{Base: s.URL, Key: "test-secret", HTTP: &http.Client{Timeout: time.Second}}
}

func TestVolumeIdentity(t *testing.T) {
	v := []Volume{{ID: "a", Name: "same"}, {ID: "b", Name: "same"}}
	if _, e := resolveVolume(v, "same"); e == nil {
		t.Fatal("ambiguous name accepted")
	}
	x, e := resolveVolume(v, "b")
	if e != nil || x.ID != "b" {
		t.Fatal(x, e)
	}
}

func TestCheapestCompatible(t *testing.T) {
	makeGPU := func(id, dc string, memory int, price float64) GPU {
		g := GPU{ID: id, Memory: memory, Manufacturer: "NVIDIA", Secure: true}
		g.Price.Secure = price
		g.DataCenters = append(g.DataCenters, struct {
			ID           string `json:"id"`
			Availability string `json:"availability"`
		}{dc, "LOW"})
		return g
	}
	in := []GPU{makeGPU("cheap-small", "EU-RO-1", 16, .1), makeGPU("wrong-dc", "US-KS-2", 48, .2), makeGPU("expensive", "EU-RO-1", 24, .8), makeGPU("winner", "EU-RO-1", 48, .4)}
	got := eligible(in, "EU-RO-1", 24, .6)
	if len(got) != 1 || got[0].ID != "winner" {
		t.Fatalf("wrong candidate: %+v", got)
	}
}

func TestPaginationAndNoRetryOnCreate(t *testing.T) {
	posts := 0
	c := testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Authorization") != "Bearer test-secret" {
			t.Error("missing auth")
		}
		if r.Method == "POST" {
			posts++
			w.WriteHeader(503)
			fmt.Fprint(w, `{"detail":"test-secret unavailable"}`)
			return
		}
		if r.URL.Query().Get("cursor") == "next" {
			fmt.Fprint(w, `{"networkVolumes":[{"id":"b"}],"pagination":{"hasNextPage":false}}`)
		} else {
			fmt.Fprint(w, `{"networkVolumes":[{"id":"a"}],"pagination":{"hasNextPage":true,"nextCursor":"next"}}`)
		}
	})
	v, e := c.volumes(context.Background())
	if e != nil || len(v) != 2 {
		t.Fatal(v, e)
	}
	e = c.request(context.Background(), "POST", "/pods", map[string]string{"name": "test"}, nil)
	if e == nil || posts != 1 || strings.Contains(e.Error(), "test-secret") {
		t.Fatal(posts, e)
	}
}

func TestPaginationLoopRejected(t *testing.T) {
	c := testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"networkVolumes":[],"pagination":{"hasNextPage":true,"nextCursor":"same"}}`)
	})
	if _, e := c.volumes(context.Background()); e == nil {
		t.Fatal("pagination loop accepted")
	}
}

func TestRecoverAmbiguousCreationWithoutDuplicate(t *testing.T) {
	posts := 0
	name := ""
	var requested map[string]any
	c := testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == "POST":
			posts++
			_ = json.NewDecoder(r.Body).Decode(&requested)
			name = requested["name"].(string)
			w.WriteHeader(503)
			fmt.Fprint(w, `{"detail":"upstream timed out"}`)
		case r.URL.Path == "/network-volumes":
			fmt.Fprint(w, `{"networkVolumes":[{"id":"vol","name":"volume-test","dataCenter":"EU-RO-1"}]}`)
		case r.URL.Path == "/catalog/gpus":
			fmt.Fprint(w, `{"gpus":[{"id":"NVIDIA example","manufacturer":"NVIDIA","memory":24,"secure":true,"price":{"secure":0.4},"dataCenters":[{"id":"EU-RO-1","availability":"LOW"}]}]}`)
		case r.URL.Path == "/pods":
			if name == "" {
				fmt.Fprint(w, `{"pods":[]}`)
				return
			}
			fmt.Fprintf(w, `{"pods":[{"id":"created","name":%q,"status":"RUNNING"}]}`, name)
		case r.URL.Path == "/pods/created":
			fmt.Fprintf(w, `{"id":"created","name":%q,"status":"RUNNING"}`, name)
		default:
			t.Error("unexpected request", r.URL.Path)
			http.NotFound(w, r)
		}
	})
	dir := t.TempDir()
	key := filepath.Join(dir, "key")
	_ = os.WriteFile(key, []byte("test"), 0600)
	_ = os.WriteFile(key+".pub", []byte("ssh-ed25519 fake"), 0600)
	a := &App{Dir: dir, api: c, Config: Config{Image: "test-image", SSHKey: key, MinVRAM: 24, MinRAM: 32, MinCUDA: "13.0", Disk: 40, Context: 16384}}
	if _, e := a.up(context.Background(), "vol", "small", false, true); e == nil {
		t.Fatal("ambiguous create should fail")
	}
	if a.State.PendingName == "" {
		t.Fatal("operation identity lost")
	}
	if _, e := a.up(context.Background(), "", "small", true, true); e != nil {
		t.Fatal(e)
	}
	if posts != 1 || a.State.PodID != "created" {
		t.Fatal(posts, a.State)
	}
	if requested["cloud"] != "SECURE" {
		t.Fatal("wrong cloud")
	}
	gpu := requested["gpu"].(map[string]any)
	if gpu["count"] != float64(1) || gpu["minRamPerGpu"] != float64(32) {
		t.Fatal(gpu)
	}
}

func TestDownOnlyDeletesPodAndVerifiesVolume(t *testing.T) {
	deleted := false
	volumeChecked := false
	c := testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		switch {
		case r.Method == "DELETE":
			if r.URL.Path != "/pods/p" {
				t.Fatal("attempted delete outside pod", r.URL.Path)
			}
			deleted = true
			w.WriteHeader(204)
		case r.URL.Path == "/pods/p":
			if deleted {
				w.WriteHeader(404)
				return
			}
			fmt.Fprint(w, `{"id":"p","name":"gpu-test","status":"RUNNING","mounts":{"network":[{"volumeId":"v","path":"/workspace"}]}}`)
		case r.URL.Path == "/network-volumes/v":
			volumeChecked = true
			fmt.Fprint(w, `{"id":"v","name":"volume-test","size":200}`)
		default:
			t.Fatal("unexpected request", r.URL.Path)
		}
	})
	a := &App{Dir: t.TempDir(), api: c, State: State{PodID: "p", VolumeID: "v", Managed: true}}
	if e := a.down(context.Background()); e != nil {
		t.Fatal(e)
	}
	if !deleted || !volumeChecked || a.State.PodID != "" {
		t.Fatal(deleted, volumeChecked, a.State)
	}
}

func TestPathsAndShellQuoting(t *testing.T) {
	for _, bad := range []string{"../../etc", "vol:../etc", "/root", "home:a\nb"} {
		if _, e := remotePath(bad); e == nil {
			t.Fatal("accepted", bad)
		}
	}
	p, e := remotePath("home:project with spaces")
	if e != nil || p != "/home/gpu/project with spaces" {
		t.Fatal(p, e)
	}
	input := []string{"spaces here", "quote'back", "$(touch /tmp/gpu-test-should-not-exist)", "semi;colon", ""}
	script := "printf '%s\\n' " + shellWords(input)
	out, e := exec.Command("bash", "-c", script).Output()
	if e != nil {
		t.Fatal(e)
	}
	if string(out) != strings.Join(input, "\n")+"\n" {
		t.Fatalf("arguments changed: %q", out)
	}
}

func TestSSELogs(t *testing.T) {
	c := testAPI(t, func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		fmt.Fprint(w, "id: 1\ndata: {\"source\":\"container\",\"line\":\"ready\"}\n\n")
	})
	var out strings.Builder
	if e := c.logs(context.Background(), "p", &out); e != nil {
		t.Fatal(e)
	}
	if out.String() != "[container] ready\n" {
		t.Fatal(out.String())
	}
}

func TestProfileLock(t *testing.T) {
	dir := t.TempDir()
	a := &App{Dir: dir}
	b := &App{Dir: dir}
	if e := a.acquire(); e != nil {
		t.Fatal(e)
	}
	defer a.release()
	if e := b.acquire(); e == nil {
		b.release()
		t.Fatal("concurrent mutation allowed")
	}
}
