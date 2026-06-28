#!/usr/bin/env bash
# ============================================================
# build-and-push.sh - Guideng Docker multi-arch image builder
# ============================================================
#
# Build backends:
#   1. Depot.dev (fast, requires depot CLI)
#   2. Docker Buildx (general, included with Docker)
#
# Usage:
#   ./build-and-push.sh
#   TAG=v0.1.0 ./build-and-push.sh
#   PUSH=0 ./build-and-push.sh
#   BUILDER=buildx ./build-and-push.sh
#   BUILDER=depot ./build-and-push.sh
#   REPO=myuser TAG=v0.1.0 ./build-and-push.sh
#   IMAGES=server ./build-and-push.sh
#   IMAGES=client ./build-and-push.sh
#
# Environment:
#   REPO                    Docker registry namespace (default: facilisvelox)
#   IMAGE_PREFIX            Image name prefix (default: guideng)
#   TAG                     Image tag (default: latest)
#   PUSH                    Push to registry, 1=yes 0=build only (default: 1)
#   BUILDER                 depot|buildx|auto (default: auto)
#   PLATFORM                Target platforms (default: linux/amd64,linux/arm64)
#   IMAGES                  server|client|all (default: all)
#   VITE_DEFAULT_SERVER_URL Build-time default client server URL (default: empty)
#
# ============================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

REPO="${REPO:-facilisvelox}"
IMAGE_PREFIX="${IMAGE_PREFIX:-guideng}"
TAG="${TAG:-latest}"
PUSH="${PUSH:-1}"
BUILDER="${BUILDER:-auto}"
PLATFORM="${PLATFORM:-linux/amd64,linux/arm64}"
IMAGES="${IMAGES:-all}"
VITE_DEFAULT_SERVER_URL="${VITE_DEFAULT_SERVER_URL:-}"

SERVER_IMAGE="${REPO}/${IMAGE_PREFIX}-server"
CLIENT_IMAGE="${REPO}/${IMAGE_PREFIX}-client"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log() { echo -e "${CYAN}[归灯]${NC} $*"; }
ok() { echo -e "${GREEN}[✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
fail() { echo -e "${RED}[✗]${NC} $*"; exit 1; }

select_builder() {
  if [[ "$BUILDER" == "depot" ]]; then
    if ! command -v depot &> /dev/null; then
      fail "'depot' command not found. Install it or use BUILDER=buildx"
    fi
    echo "depot"
  elif [[ "$BUILDER" == "buildx" ]]; then
    if ! command -v docker &> /dev/null; then
      fail "'docker' command not found"
    fi
    echo "buildx"
  else
    if command -v depot &> /dev/null; then
      echo "depot"
    elif command -v docker &> /dev/null; then
      echo "buildx"
    else
      fail "Neither 'depot' nor 'docker' was found"
    fi
  fi
}

SELECTED_BUILDER="$(select_builder)"

setup_buildx() {
  if ! docker buildx inspect guideng-builder &> /dev/null 2>&1; then
    log "Creating buildx builder: guideng-builder"
    docker buildx create --name guideng-builder --use --driver docker-container
  else
    docker buildx use guideng-builder
  fi
}

ALSO_LATEST="no"
[[ "$TAG" != "latest" ]] && ALSO_LATEST="yes"

build_and_push() {
  local image="$1"
  local context="$2"
  local also_latest="$3"
  shift 3

  local tags=("-t" "${image}:${TAG}")
  [[ "$also_latest" == "yes" ]] && tags+=("-t" "${image}:latest")
  local build_args=("${tags[@]}")
  if [[ "$#" -gt 0 ]]; then
    build_args+=("$@")
  fi

  local tag_display="${image}:${TAG}"
  [[ "$also_latest" == "yes" ]] && tag_display="${tag_display}, ${image}:latest"

  log "Image: ${tag_display}"
  log "Context: ${context}"
  log "Platforms: ${PLATFORM}"
  log "Builder: ${SELECTED_BUILDER}"

  if [[ "$SELECTED_BUILDER" == "depot" ]]; then
    if [[ "$PUSH" == "1" ]]; then
      log "Mode: build + push (depot)"
      depot build \
        --platform "$PLATFORM" \
        "${build_args[@]}" \
        --push \
        "$context"
    else
      log "Mode: build only (depot)"
      depot build \
        --platform "$PLATFORM" \
        "${build_args[@]}" \
        --load \
        "$context"
    fi
  else
    setup_buildx

    if [[ "$PUSH" == "1" ]]; then
      log "Mode: build + push (buildx)"
      docker buildx build \
        --platform "$PLATFORM" \
        "${build_args[@]}" \
        --push \
        "$context"
    else
      log "Mode: build only (buildx)"
      if [[ "$PLATFORM" == *","* ]]; then
        warn "Multi-platform build without push cannot be loaded into local Docker; validating build only"
        docker buildx build \
          --platform "$PLATFORM" \
          "${build_args[@]}" \
          "$context"
      else
        docker buildx build \
          --platform "$PLATFORM" \
          "${build_args[@]}" \
          --load \
          "$context"
      fi
    fi
  fi
}

should_build() {
  local name="$1"
  [[ "$IMAGES" == "all" || "$IMAGES" == "$name" ]]
}

echo ""
echo "  +---------------------------------------+"
echo "  |   Guideng - Docker image builder      |"
echo "  +---------------------------------------+"
echo ""
log "Repository:  ${REPO}"
log "Image prefix:${IMAGE_PREFIX}"
log "Tag:         ${TAG}"
log "Push:        $([ "$PUSH" == "1" ] && echo "yes" || echo "no")"
log "Builder:     ${SELECTED_BUILDER}"
log "Platforms:   ${PLATFORM}"
log "Images:      ${IMAGES}"
echo ""

if should_build "server"; then
  build_and_push "$SERVER_IMAGE" "${ROOT_DIR}/server" "$ALSO_LATEST"
  echo ""
fi

if should_build "client"; then
  build_and_push \
    "$CLIENT_IMAGE" \
    "${ROOT_DIR}/client" \
    "$ALSO_LATEST" \
    --build-arg "VITE_DEFAULT_SERVER_URL=${VITE_DEFAULT_SERVER_URL}"
  echo ""
fi

if ! should_build "server" && ! should_build "client"; then
  fail "IMAGES must be one of: all, server, client"
fi

ok "Build finished"
echo ""

if [[ "$PUSH" == "1" ]]; then
  ok "Images pushed:"
  should_build "server" && echo "   docker pull ${SERVER_IMAGE}:${TAG}"
  should_build "client" && echo "   docker pull ${CLIENT_IMAGE}:${TAG}"
  if [[ "$ALSO_LATEST" == "yes" ]]; then
    should_build "server" && echo "   docker pull ${SERVER_IMAGE}:latest"
    should_build "client" && echo "   docker pull ${CLIENT_IMAGE}:latest"
  fi
else
  ok "Images built:"
  should_build "server" && echo "   docker images ${SERVER_IMAGE}"
  should_build "client" && echo "   docker images ${CLIENT_IMAGE}"
fi

echo ""
echo "  Run locally:"
echo "   docker compose up -d"
echo ""
echo "  Example:"
echo "   TAG=v0.1.0 REPO=${REPO} ./build-and-push.sh"
echo "   PUSH=0 PLATFORM=linux/amd64 ./build-and-push.sh"
echo ""
