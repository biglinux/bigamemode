# Continuous integration

Every push and pull request answers one question: **does this commit build,
pass its tests and produce an installable Arch/BigLinux package?** Nothing in
the application or the package depends on this page.

## Workflows

```text
pull_request ──► "Package check" (package-check.yml) ─┐
                                                      │ workflow_call
push main,                                            ▼
testing-*, stable-* ──► "Build Package" (build-package.yml)
                         ├─ rust (backend-tests.yml, workflow_call):
                         │    fmt · clippy · test · test on a baseline CPU ·
                         │    doc · authorization · translations
                         └─ package: source of $GITHUB_SHA → makepkg -s
                              → identify → metadata → contents → legacy CPUs
                              → desktop → AppStream → namcap → SHA256SUMS
                              → artifact
```

One run per push or pull request, with both jobs in parallel.
`backend-tests.yml` runs only when called, or by hand.

### How the BigLinux builders are triggered

The BigLinux package builders build a branch of biglinux/bigamemode when a run
of the workflow named **Build Package** ends in **success**, with that run's
branch. No step sends anything, and there is no secret. Hence:

- **build-package.yml** ("Build Package") runs only for branches the builders
  can build: pushes to plain branch names (`main`, `testing-*`, `stable-*`;
  `'*'` does not match `/`) and manual runs. A manual run of a branch with `/`
  fails at once.
- **package-check.yml** ("Package check") runs the same jobs for pull requests
  and manual runs of any branch, through `workflow_call`. Under that name a
  success is not handed over.

A `repository_dispatch` step to the builders was tried once: its token was
refused, the failed job failed the run, and no push was built after it.

### backend-tests.yml: the code

Runs in `archlinux:base-devel` with `--init` (the launcher's terminate test
needs a PID 1 that reaps orphans), as the ordinary user `builder`: the tests
of permission refusals pass under root for the wrong reason. Every check runs
when the setup succeeded, so the first failure does not hide the others.

| Step | Command |
|---|---|
| Formatting | `cargo fmt --all --check` |
| Clippy | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings -A clippy::assert_is_empty` |
| Tests | `cargo test --locked --workspace` |
| Tests on a baseline x86-64 CPU | the same tests under `qemu-x86_64 -cpu Opteron_G1` (SSE2 only), see [CPU-COMPATIBILITY.md](CPU-COMPATIBILITY.md) |
| Documentation | `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps` |
| Helper authorization | `tests/daemon-authorization.sh` on a private `dbus-daemon`, no Polkit: every privileged method must answer *Access denied* |
| Translations | `locale/extract-strings.py --check` (template current, `LINGUAS` complete), `msgfmt --check --check-format` on every catalogue |

`--locked` everywhere: CI never rewrites `Cargo.lock`.
`clippy::assert_is_empty` is the one allowed lint: it would rewrite test
assertions such as `assert!(x.is_empty())`, and it is allowed on the command
line because an older Clippy rejects the unknown name.

### build-package.yml: the package

1. **Source of this commit.** `.github/scripts/prepare-package-source.sh`
   copies `pkgbuild/` as committed, packs `HEAD` with `git archive` and
   appends `source=(<tarball>) sha256sums=(<sum>)` to the copy, so
   `prepare()`, `build()`, `check()` and `package()` run exactly as written.
   It refuses a recipe with a second source and checks the override with
   `makepkg --printsrcinfo`. `pkgbuild/PKGBUILD` itself still builds `main`.
2. **Build.** `makepkg --syncdeps --cleanbuild --noconfirm --log` as
   `builder`, whose sudo is limited to `pacman`: the LTO release build, the
   template check, the catalogues, `check()` and `package()`.
3. **Identify** the package and `bigame-mode-debug` with
   `makepkg --packagelist` and `pacman -Qpq`.
4. **Metadata.** Name, version and architecture match the recipe; the
   `[workspace.package]` version in `Cargo.toml` and the newest `<release>` in
   the AppStream file equal `pkgver`.
5. **Contents.** `.github/scripts/check-package-contents.sh`: every runtime
   file with its mode and owner, a catalogue per `locale/LINGUAS` entry,
   nothing outside `/etc` and `/usr`, and the references between the files
   (desktop `Exec`/`Icon`, D-Bus activation → systemd unit → binary, bus
   policy → bus name, AppStream → desktop entry).
6. **Legacy CPUs.** `.github/scripts/legacy-cpu-test.sh` runs the packaged
   binaries on emulated processors, see
   [CPU-COMPATIBILITY.md](CPU-COMPATIBILITY.md).
7. **Desktop entry and AppStream.** `desktop-file-validate`, and
   `appstreamcli validate --no-net`; every AppStream `<url>` must be
   `https://github.com/biglinux/bigamemode[/…]`.
8. **namcap** on `pkgbuild/PKGBUILD` and the package. Errors fail the build;
   warnings go to the summary.
9. **Artifact** `bigame-mode-<pkgver>-<pkgrel>-<commit>`: both packages,
   `SHA256SUMS`, `namcap.log`, `package-info.txt`, `package-files.txt` and
   makepkg's logs per stage. Nothing is published from CI.

Permissions are `contents: read`, no secrets, no `pull_request_target`. The
only third-party actions are `actions/checkout` and `actions/upload-artifact`,
pinned by commit. There is no cache (Arch's `rust` moves with every stable
release, and `--cleanbuild` starts clean anyway) and no path filter (nearly
every path ends up in the package).

### namcap: expected warnings

| Message | Why |
|---|---|
| `Directory (etc/falcond) is empty`, `Directory (usr/share/falcond/profiles/user) is empty` | The package owns them so the helper's sandbox can write there before falcond is installed. |
| `Dependency cairo / pango / graphene / gdk-pixbuf2 detected and implicitly satisfied` | `bigame-ui` links them through gtk4-rs; `gtk4` pulls them in. |
| `Dependency included, but may not be needed ('dbus', 'polkit', 'systemd', 'curl', 'libarchive', 'hwdata', 'iputils', 'iproute2')` | namcap sees only ELF links. These are services the helper runs under, programs Big Game Mode runs (`curl`, `bsdtar`, `ping`, `tc`) and data it reads (`pci.ids`). |

A new `W:` line should be read and either fixed or added here.

## Reproducing locally

On BigLinux/Arch with the build dependencies:

```bash
cd bigame-engine
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings -A clippy::assert_is_empty
cargo test --locked --workspace
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps
cargo build --locked --workspace --bin bigame-daemon
cd ..
bash tests/daemon-authorization.sh bigame-engine/target/debug/bigame-daemon
python3 locale/extract-strings.py --check
for po in locale/*.po; do msgfmt --check --check-format -o /dev/null "$po"; done
```

The package as CI builds it (committed content only):

```bash
dir=$(mktemp -d)/package-build
.github/scripts/prepare-package-source.sh "$dir"
(cd "$dir" && makepkg --syncdeps --cleanbuild --noconfirm --log)
pkg=$(cd "$dir" && makepkg --packagelist | grep -v -- '-debug-')
.github/scripts/check-package-contents.sh "$pkg" "$dir/package-root"
desktop-file-validate "$dir"/package-root/usr/share/applications/*.desktop
appstreamcli validate --no-net "$dir"/package-root/usr/share/metainfo/*.xml
namcap pkgbuild/PKGBUILD "$pkg"
```

`actionlint .github/workflows/*.yml` and `shellcheck .github/scripts/*.sh
tests/*.sh` check the workflows themselves.

## Investigating a failure

1. The run's **Summary** shows which check failed and which did not run.
2. `gh run view <run-id> --repo biglinux/bigamemode --log-failed` shows the
   failing step.
3. The artifacts hold `cargo-test.log`, `daemon-authorization.log`,
   `ci-versions.log` (commit, toolchain and every installed package), and
   makepkg's logs per stage with `namcap.log`.
4. A failure right after an Arch update (new rustc, Clippy lint or namcap
   rule) shows up as a version change in `ci-versions.log` against the last
   green run.

## What CI cannot test

CI runs in a container: no systemd as PID 1, no system bus, no Polkit agent,
no GPU and no games. A real BigLinux installation is needed for the helper
started by D-Bus activation inside its sandbox and granted by a Polkit prompt;
falcond taken over and handed back; `scx_loader` and power-profiles-daemon;
install, upgrade and removal through pacman; Gamescope, MangoHud, vkBasalt,
lsfg-vk, OptiScaler downloads and real games; GPU telemetry and hybrid
graphics; the GTK interface and the tray.
