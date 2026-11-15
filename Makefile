CARGO ?= cargo
IMAGE ?= felipeoperti/fastgpu-workspace:0.3.0

.PHONY: build test image install npm-package test-npm
build:
	$(CARGO) build --release --locked --target-dir target
	mkdir -p bin
	cp target/release/fastgpu bin/fastgpu
test:
	$(CARGO) fmt --all -- --check
	$(CARGO) test --locked
	$(CARGO) clippy --locked --all-targets -- -D warnings
image:
	docker build -f docker/Dockerfile -t $(IMAGE) .
install: build
	install -D -m 755 bin/fastgpu $(DESTDIR)$(HOME)/.local/bin/fastgpu
npm-package:
	mkdir -p dist
	npm pack --pack-destination dist
test-npm:
	npm test
