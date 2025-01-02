REGISTRY?=registry.h.jw910731.dev/nix
IMAGE=ntnu-course-bot
VERSION=0.1.0

TAG=$(REGISTRY)/$(IMAGE):$(VERSION)

.PHONY: all docker-build

all: docker-build

docker-build:
	docker build . --tag $(TAG)
