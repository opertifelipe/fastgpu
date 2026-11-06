package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"syscall"

	"github.com/opertifelipe/gpu-extension/internal/fastgpu"
)

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := fastgpu.Execute(ctx); err != nil {
		fmt.Fprintln(os.Stderr, "fastgpu:", err)
		var e *exec.ExitError
		if errors.As(err, &e) && e.ExitCode() > 0 {
			os.Exit(e.ExitCode())
		}
		os.Exit(1)
	}
}
