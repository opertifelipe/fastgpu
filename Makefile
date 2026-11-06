GO ?= go
IMAGE ?= felipeoperti/fastgpu-workspace:0.2.1

.PHONY: build test image install
build:
	mkdir -p bin
	CGO_ENABLED=0 $(GO) build -trimpath -o bin/fastgpu ./cmd/fastgpu
test:
	$(GO) test -race ./...
	$(GO) vet ./...
image:
	docker build -f docker/Dockerfile -t $(IMAGE) .
install: build
	install -D -m 755 bin/fastgpu $(DESTDIR)$(HOME)/.local/bin/fastgpu
