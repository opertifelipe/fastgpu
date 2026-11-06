package fastgpu

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"text/tabwriter"
)

const ModelAlias = "qwen3.8-27b"

type modelPreset struct {
	File, URL, SHA256 string
	Size              int64
}

var modelPresets = map[string]modelPreset{
	ModelAlias: {
		File:   "Qwen3.8-27B-UD-Q4_K_M.gguf",
		URL:    "https://huggingface.co/unsloth/Qwen3.8-27B-GGUF/resolve/4ca720788d1e01f1bff70c033e0d0028fd02e502/Qwen3.8-27B-UD-Q4_K_M.gguf",
		SHA256: "322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482",
		Size:   16464440224,
	},
	"nanbeige4.3-33b": {
		File:   "Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf",
		URL:    "https://huggingface.co/bartowski/Nanbeige_Nanbeige4.3-33B-GGUF/resolve/9beb68421fdf7be095093a6888325fa2c83ddd38/Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf",
		SHA256: "2cef79855ca47ee6571e166db32a07089e34f3cf78e0397b262d124ddb814c37",
		Size:   20788825216,
	},
}

func presetAliases() []string {
	names := make([]string, 0, len(modelPresets))
	for name := range modelPresets {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

func listModels(out io.Writer, volume, directory string) error {
	root, err := filepath.EvalSymlinks(volume)
	if err != nil {
		return err
	}
	dir, err := persistentPath(root, directory)
	if err != nil {
		return err
	}
	w := tabwriter.NewWriter(out, 0, 4, 2, ' ', 0)
	count := 0
	err = filepath.WalkDir(dir, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.Type().IsRegular() || !strings.HasSuffix(strings.ToLower(entry.Name()), ".gguf") {
			return nil
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		target := quote(path)
		for alias, preset := range modelPresets {
			if path == filepath.Join(root, ".gpu", "models", preset.File) {
				target = alias
				break
			}
		}
		if count == 0 {
			fmt.Fprintln(w, "DEPLOY TARGET\tSIZE (GB)")
		}
		fmt.Fprintf(w, "%s\t%.2f\n", target, float64(info.Size())/1e9)
		count++
		return nil
	})
	if err != nil {
		return err
	}
	if count == 0 {
		_, err = fmt.Fprintln(out, "No GGUF models found in", dir)
		return err
	}
	if err = w.Flush(); err != nil {
		return err
	}
	_, err = fmt.Fprintln(out, "\nDeploy with: fastgpu deploy <DEPLOY TARGET>")
	return err
}
