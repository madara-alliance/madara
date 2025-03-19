# Using Dev Containers with Madara

## Prerequisites

- [Docker](https://www.docker.com/products/docker-desktop/)
- [VS Code](https://code.visualstudio.com/download) with the [Dev Containers extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)

## Quick Start

1. **Clone the repository:**

   ```bash
   git clone https://github.com/madara-alliance/madara.git
   cd madara
   ```

2. **Open in Dev Container:**
   - In VS Code, click the `><` in the bottom-left corner
   - Select "Reopen in Container"
   - VS Code will build and start the dev container with all dependencies pre-installed

## Features

- Pre-configured Rust development environment
- Python, Node, Rust already installed
- Consistent development environment across team members

## Troubleshooting

### IDE is stuck on `Installing Server`

Sadly this step takes a long time.
Be patient and it *should* eventually install the backend into the container to allow it to communicate to VSCode.

### Dev Container Requires Update

If a dependency needs to be added, or an environment version needs to change:

- Update the **Dockerfile** in `<repo>/.devcontainer/Dockerfile`
- Open CMD Pallet (Ctrl-P) and select `>Dev Containers: Rebuild Container`

### Problems Loading Dev Container

- Open CMD Pallet (Ctrl-P) and select `>Dev Containers: Show Container Log`

## Learn More

- [VSCode Dev Containers documentation](https://code.visualstudio.com/docs/remote/containers)
- [Dev Containers](https://containers.dev/)
