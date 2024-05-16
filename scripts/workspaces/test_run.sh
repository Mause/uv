#!/bin/bash

set -ex

# --offline --find-links "$(git rev-parse --show-toplevel)/scripts/workspaces/vendored"

(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-in-example/examples/bird-feeder" && \
  uv venv && \
  cargo run --profile fast-build -- run --preview check_installed_bird_feeder.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-in-example" && \
  uv venv && \
  cargo run --profile fast-build -- run --preview check_installed_albatross.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-just-project" && \
  uv venv && \
  cargo run --profile fast-build -- run --preview check_installed_albatross.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-project-in-excluded/excluded/bird-feeder" && \
  uv venv && \
  cargo run --profile fast-build -- run --preview check_installed_bird_feeder.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-root-workspace" && \
  uv venv && \
  cargo run --profile fast-build -- run --preview check_installed_albatross.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-root-workspace/packages/bird-feeder" && \
  uv venv ../../.venv && \
  cargo run --profile fast-build -- run --preview check_installed_bird_feeder.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-virtual-workspace/packages/albatross" && \
  uv venv ../../.venv && \
  cargo run --profile fast-build -- run --preview check_installed_albatross.py
)
(
  cd "$(git rev-parse --show-toplevel)/scripts/workspaces/albatross-virtual-workspace/packages/bird-feeder" && \
  uv venv ../../.venv && \
  cargo run --profile fast-build -- run --preview check_installed_bird_feeder.py
)
