# AKSprayPaint – Agent Instructions

## 🎯 Project Purpose
**AKSprayPaint** is a Rust‑based utility that changes the colors of the current wallpaper to match the noctalia theme. The binary produced is `akspraypaint`.

---

## 🏗️ Architecture Overview
```
AKSprayPaint/
├─ Cargo.toml                # Crate metadata, version, dependencies
├─ src/
│  ├─ main.rs                # Entry point – CLI argument parsing & dispatch
│  ├─ lib.rs                 # Core library (shared logic)
│  ├─ commands/              # Sub‑commands (e.g., `mix`, `profile`, `run`)
│  │   ├─ mod.rs
│  │   ├─ mix.rs
│  │   ├─ profile.rs
│  │   └─ run.rs
│  └─ utils/                 # Helper modules (config handling, logging, etc.)
│      ├─ config.rs
│      └─ logger.rs
├─ .github/
│  └─ workflows/
│      └─ build.yml          # GitHub Actions CI (cargo build –release)
├─ README.md
├─ AGENTS.md                 # <‑‑ you are reading this file
└─ .secrets                  # (ignored) – runtime secrets (see below)
```

*The repository is deliberately minimal – all business logic lives under `src/`.  The CI pipeline builds the binary automatically; developers **must not** install Rust locally for production builds.*

---

## 📦 Key Files & Their Roles
| File/Directory | Description |
|---------------|-------------|
| `Cargo.toml` | Crate metadata, version number, dependency list. |
| `src/main.rs` | CLI entry point – uses `clap` (or similar) to route sub‑commands. |
| `src/lib.rs` | Re‑usable library functions shared across commands. |
| `src/commands/` | Individual command implementations (`mix`, `profile`, `run`, …). |
| `src/utils/` | Utility modules (configuration parsing, logging, error handling). |
| `.github/workflows/build.yml` | GitHub Actions workflow that runs `cargo build --release`. |
| `README.md` | User‑facing documentation – **must be kept up‑to‑date** when features change. |
| `.secrets` (runtime only) | Holds runtime secrets (e.g., API keys). **Never commit**. |
| `AGENTS.md` | This instruction file for AI assistants. |

---

## 🛠️ Build & Release Process
| Step | Command / Action | Notes |
|------|------------------|-------|
| **Local verification (optional)** | `cargo check` | Checks syntax without producing a binary. |
| **Full build** | `cargo build --release` | Produces `target/release/akspraypaint`. |
| **Version bump** | Edit `Cargo.toml` → update `version = "x.y.z"` | The CI will automatically tag the commit if the version changed. |
| **Homebrew formula** | Update the tap `Akinus21/homebrew-tap` with the new version and SHA256. | Formula lives in the tap repo; see its `README` for contribution steps. |
| **Publish** | Push to `main` → GitHub Actions builds and uploads the binary as an artifact. | No manual upload required. |

**Important:** Do **not** run `cargo install` locally for production; rely on the CI artifact or Homebrew tap.

---

## 🔐 Authentication & Secrets

| Item | Location / Value |
|------|-----------------|
| **SSH key for Git** | `/config/.ssh/github` |
| **Runtime secrets file** | `/home/akinus/dockge-stacks/dev-stack/.secrets` |
| **GitHub webhook secret** | `4d82982b0a0010a706a40cf272f49c9ddfee93162a2c4b714eebc6ded10038f5` |
| **Webhook URL** | `https://webhook.akinus21.com/webhook/akspraypaint-build` |
| **Homebrew tap** | `Akinus21/homebrew-tap` |

*Never expose these values in commits, PRs, or logs.*

---

## 📤 Git Push Workflow (SSH)

```bash
# From the project root
cd /home/akinus/dockge-stacks/dev-stack/projects/AKSprayPaint

# Stage & commit
git add -A
git commit -m "<concise description of changes>"

# Push using the dedicated SSH key
GIT_SSH_COMMAND="ssh -i /config/.ssh/github -o StrictHostKeyChecking=no" \
git push origin main
```

- **Do not** use `gh auth login`; the CLI is not authenticated in this environment.
- Always push **after** you have verified the changes (lint, `cargo check`, unit tests).

---

## 📚 Documentation Conventions

1. **README.md**  
   - Update the **Installation** section when the binary name, path, or Homebrew formula changes.  
   - Add a **Usage** example for every new sub‑command.  
   - Keep the **Version** badge in sync with `Cargo.toml`.

2. **Code Comments**  
   - Use `///` for public API docs; `//` for internal notes.  
   - Document any environment variables read from `.secrets`.

3. **Changelog**  
   - Maintain a `CHANGELOG.md` (or use `git log --oneline` in releases).  
   - Follow [Keep a Changelog](https://keepachangelog.com/) style.

4. **CI / Webhook**  
   - The webhook endpoint (`https://webhook.akinus21.com/webhook/akspraypaint-build`) expects a POST with the build status.  
   - Do **not** modify the webhook secret; it is set via `gh secret set WEBHOOK_SECRET`.

---

## 🧩 Development Conventions

| Convention | Detail |
|-----------|--------|
| **Branching** | Work directly on `main` for small fixes. For larger features, create a branch `feature/<name>` and open a PR. |
| **Formatting** | Run `cargo fmt` before committing. |
| **Linting** | Run `cargo clippy -- -D warnings` to enforce no warnings. |
| **Testing** | Add unit tests in the same file (`#[cfg(test)]`). Run `cargo test`. |
| **Secrets** | Access runtime secrets via the `secrets` module (reads from the file pointed to by `$SECRETS_PATH`). Do **not** hard‑code any secret. |
| **Binary Output** | After `cargo build --release`, the binary is at `target/release/akspraypaint`. Do not rename it locally. |

---

## 🚀 Quick Start (for the AI Assistant)

1. **Clone (if needed)**  
   ```bash
   git clone git@github.com:Akinus21/AKSprayPaint.git \
       /home/akinus/dockge-stacks/dev-stack/projects/AKSprayPaint
   ```

2. **Make a change** (e.g., add a new sub‑command).  
3. **Run static checks**  
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   cargo test
   ```

4. **Commit & push** using the SSH workflow above.  

The CI will:
- Build `cargo build --release`
- Upload the binary as an artifact
- Trigger the webhook at `https://webhook.akinus21.com/webhook/akspraypaint-build`

---

*End of AGENTS.md – follow these instructions to keep AKSprayPaint healthy, secure, and continuously deployable.*