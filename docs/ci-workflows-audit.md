# CI workflows: audit and design

This page records why `.github/workflows/backend-tests.yml` and
`.github/workflows/build-package.yml` look the way they do, how to reproduce
what they do, and how to read a failure. Nothing in the application or the
package depends on it.

Every pull request should answer one question: **does this code build, pass
its tests and produce an installable Arch/BigLinux package?** The two
workflows answer it for the exact commit under test.

## Where the workflows came from

| Commit | What happened |
|---|---|
| `a60ef91` | `integration.yml`: Rust checks and a `makepkg` build of the checkout. |
| `7dd3d99` | `build-package.yml`: the BigLinux builders' hook. Every push sent a `repository_dispatch` to `BigLinux-Package-Build/build-package`. |
| `a663783` | The hook reverted to the organization's template (secrets interpolated into the script, no `--fail`, an ARM request for an x86_64-only package); `integration.yml` deleted. |
| `30cee92` | Both files replaced by big-video-converter's (Python, pytest, Debian, Xvfb). The builders' hook disappeared. |
| `72fed17` | The copies adapted to Rust and Arch again. Green, but with the problems below. |

## Findings

The runs before this change were green; the problems were in what green
meant.

| Problem | Root cause | Fix | How it is validated |
|---|---|---|---|
| ~~Pushes to `main`, `testing-*` and `stable-*` no longer reached the BigLinux package builders.~~ **Wrong**: see *How the BigLinux builders are triggered* below. | — | The `notify-builders` job added for it is removed. | |
| `if: always()` with `if-no-files-found: error` on the package upload. | Copied pattern. | The package is uploaded only on success; after a failure a separate upload keeps whatever logs exist, with `if-no-files-found: ignore`. | A failure shows one error: the step that failed. |
| A change in cargo's output format could fail the backend run. | The totals parser raised an error when it found no `test result:` line. | Totals are shown in the summary only; cargo's exit status decides. | |
| The first failing check hid the others (rustfmt failing skipped Clippy and the tests). | Default step semantics. | Every check runs when the setup succeeded (`!cancelled() && steps.setup.outcome == 'success'`); any failure still fails the job. | |
| `build-package` ran on push to every branch **and** on the pull request of the same branch, with no concurrency limit. | `branches: ['**']` from big-video-converter. | `branches: ['*']` (no `/`: `main`, `testing-*`, `stable-*`, as the builders' hook always had) plus `pull_request`; superseded pull request runs are cancelled. | |
| namcap ran on the CI copy of the recipe (with the source override), not on `pkgbuild/PKGBUILD`. | Path. | `namcap ../pkgbuild/PKGBUILD <package>`. | `namcap.log`. |
| The package was found with a glob that could also match `bigame-mode-debug`. | Arch's `makepkg.conf` builds a debug package (`0e2136e` patched the glob). | `makepkg --packagelist`, each file's name read with `pacman -Qpq`. | |
| `SHA256SUMS` covered the package but not the debug package uploaded with it. | | Both, with names relative to the artifact so `sha256sum -c SHA256SUMS` works after download. | |
| Only a few installed files were checked. | | `.github/scripts/check-package-contents.sh`: every runtime file with its mode and owner, a catalogue per `locale/LINGUAS` entry, nothing outside `/etc` and `/usr`, the references between the files (desktop `Exec`/`Icon`, D-Bus activation → systemd unit → binary, bus policy → bus name, AppStream → desktop entry) and well-formed XML. | Step *Validate package contents*. |
| `rust-version = "1.85"` is false. | Let chains (stable in 1.88) are used, and the locked zbus 5.14 needs 1.87. | **Not fixed here**, see *Pending*: correcting it changes Clippy's verdict on application code. | `cargo +1.85 check` fails, `cargo +1.88 check` passes. |
| The README described `integration.yml`, deleted in `a663783`. | | The *Packaging* section describes the two workflows. | |
| `graphics::optiscaler::tests::the_cache_keeps_the_releases_in_use_and_drops_the_rest` failed once on GitHub (`prune_cache(..) >= 110` after `drop(held)`), and passed locally and on the previous run. | A race in the test, not in CI. A flock belongs to the open file description, and a process another test spawns at that moment (several run `systemctl`, `ps`…) holds a copy of every descriptor until it executes: `O_CLOEXEC` closes it only then. So the lock can outlive `drop()` for an instant. A minimal program (lock, drop, `try_lock`, with or without threads spawning `true`) showed 0 of 5.8 million `try_lock`s refused without spawning, and 1.6 % refused with it (21 % on two cores). | The test retries the prune for up to a second before asserting, as the application would on its next prune. A prune that never frees anything still fails it. | `cargo test`; the assertion is unchanged. |

No application code, recipe or Cargo manifest changed. The one change
outside CI is the test above, in `bigame-core/src/graphics/optiscaler.rs`.

### Assumptions that no longer held

- **big-video-converter's environment.** Debian, Xvfb, pytest, ffmpeg and
  `dev-bigbruno` came with `30cee92`; `72fed17` removed them. Nothing in this
  project needs a display: no test opens a GTK window, and no test but the
  authorization script talks to D-Bus. The workflows install neither Xvfb nor
  a session bus.
- **Building the checkout means building `pkgbuild/`.** It does not: the
  recipe fetches `main` from GitHub. Run as is in a pull request, it would build
  `main` and pass for code it never compiled.

## Final architecture

```text
pull_request ─┬─► backend-tests ─── rust: fmt · clippy · test · doc · authorization · translations
push main ────┤
              │
              └─► package-check ──┐
                                   │ workflow_call
push main,                         ▼
testing-*, stable-* ──► build-package ─── package: prepare source of $GITHUB_SHA → makepkg -s
                                              → identify → metadata → contents → desktop
                                              → AppStream → namcap → SHA256SUMS → artifact
                           │
                           └─ a successful "Build Package" run: the BigLinux
                              builders build its branch
```

## How the BigLinux builders are triggered

The BigLinux builders (GitLab, `gitlab.bigib.org/builder/packagebuilder`)
build a branch of biglinux/bigamemode when a run of the workflow named
**Build Package** ends in **success**, with that run's branch. No step of the
workflow has to send anything. The builder's public job history against this
repository's runs, 2026-10-07 and 08 (UTC):

| Build Package run | Conclusion | Builder job |
|---|---|---|
| push `main`, `testing-2026-10-07` (`72fed17`, no dispatch step), ends 07:14 / 07:16 | success | `main` 07:14, `testing-2026-10-07` 07:16 |
| push `stable-2026-10-07`, ends 12:34 | success | `stable-2026-10-07` 12:34 |
| pull requests `fix/…` (4 runs), end 14:18, 14:32, 19:34, 19:05 | success | `fix/…` at the same minutes, failing at the recipe's `sed` (the `/` ends its `s/…/…/`) |
| pushes `main`, `testing-2026-10-07-1`, `-4`, `testing-2026-10-08` | failure (`notify-builders`: 401) | none |

So the `repository_dispatch` added on 2026-10-07 (`notify-builders`) did the
opposite of its purpose: its token is refused (401), the failed job failed the
run, and no push was built after it. Pull requests, built under the same
workflow name, were handed over instead.

Hence:

- **build-package.yml** ("Build Package") runs only for branches the builders
  can build: pushes to plain branch names (`main`, `testing-*`, `stable-*`;
  `'*'` does not match `/`) and manual runs. A manual run of a branch with `/`
  fails at once, before anything is built.
- **package-check.yml** ("Package check") runs the same job for pull requests
  and manual runs of any branch, through `workflow_call`. Under that name a
  success is not handed over.
- There is no dispatch step and no secret.

### backend-tests.yml: the code

Runs in `archlinux:base-devel` with `--init` (the launcher's terminate test
needs a PID 1 that reaps orphans), as the ordinary user `builder`: the tests
of permission refusals pass under root for the wrong reason.

| Step | Command |
|---|---|
| Check Rust formatting | `cargo fmt --all --check` |
| Run Clippy | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings -A clippy::assert_is_empty` |
| Run workspace tests | `cargo test --locked --workspace` |
| Run workspace tests on a baseline x86-64 CPU | the same tests, run by `qemu-x86_64 -cpu Opteron_G1` (SSE2 only): see [CPU-COMPATIBILITY.md](CPU-COMPATIBILITY.md) |
| Check Rust documentation | `RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps` |
| Test helper authorization | `tests/daemon-authorization.sh` on a private `dbus-daemon`, no Polkit: every privileged method must answer *Access denied* |
| Validate translations | `locale/extract-strings.py --check` (template current, `LINGUAS` complete), `msgfmt --check --check-format` on every catalogue |

`--locked` everywhere: CI never rewrites `Cargo.lock`.

`clippy::assert_is_empty` is the one allowed lint. It appeared in Clippy 1.99
and would rewrite 61 test assertions such as `assert!(x.is_empty())` into
`assert_eq!(x, [] as [String; 0])`. It is allowed on the command line, where
the README also has it, because an older Clippy rejects the unknown name. CI
uses Arch's `rust`, always the current stable release.

### build-package.yml: the package

1. **Prepare the PKGBUILD for this commit.** It fails unless the checkout is
   `$GITHUB_SHA`. For a pull request that is the merge commit
   `actions/checkout` checks out. `.github/scripts/prepare-package-source.sh`
   then copies `pkgbuild/` as committed, packs `HEAD` with
   `git archive --prefix=bigame-mode/` and appends
   `source=(<tarball>) sha256sums=(<sum>)` to the copy. `prepare()`,
   `build()`, `check()` and `package()` run exactly as written. The script
   refuses to run if the recipe gains a second source, and it checks with
   `makepkg --printsrcinfo` that the override took effect.
   `pkgbuild/PKGBUILD` itself is unchanged and still builds `main` for the
   BigLinux builders.
2. **Build Arch package.** `makepkg --syncdeps --cleanbuild --noconfirm --log`
   runs as `builder`, whose sudo is limited to `/usr/bin/pacman` (the only
   thing `--syncdeps` needs). This is the full recipe: the LTO release build,
   the translation template check, the catalogues, `check()` (every test in
   release mode) and `package()`.
3. **Identify the built packages.** `makepkg --packagelist`; the package and
   `bigame-mode-debug` are told apart by name.
4. **Validate package metadata.** `pacman -Qip`/`-Qlp` are saved to
   `package-info.txt`/`package-files.txt`. Name, version and architecture must
   match the recipe. The `[workspace.package]` version in `Cargo.toml` and the
   newest `<release>` in the AppStream file must equal `pkgver`. None of these
   values is written into the workflow.
5. **Validate package contents.** `.github/scripts/check-package-contents.sh`
   (see the table above).
6. **Check CPU compatibility (legacy x86-64).** The packaged binaries, run by
   `.github/scripts/legacy-cpu-test.sh` on emulated processors from the
   x86-64 baseline up. See [CPU-COMPATIBILITY.md](CPU-COMPATIBILITY.md).
7. **Validate desktop entry.** `desktop-file-validate` on the installed entry,
   with the catalogues merged in.
8. **Validate AppStream metadata.** `appstreamcli validate --no-net`, and every
   `<url>` must be `https://github.com/biglinux/bigamemode[/…]`.
9. **Run namcap.** On `pkgbuild/PKGBUILD` and the package. Errors fail the
   build; warnings go to the summary (see the next section).
10. **SHA256SUMS** for the package and the debug package.
11. **Artifact** `bigame-mode-<pkgver>-<pkgrel>-<commit>`: the package, the
    debug package, `SHA256SUMS`, `namcap.log`, `package-info.txt`,
    `package-files.txt`, `build-report.md` and makepkg's logs per stage.
12. **Handing over.** None in the workflow: the run's success is what the
    BigLinux builders act on (see *How the BigLinux builders are
    triggered*).

### namcap: what it reports today

| Message | Class | Why |
|---|---|---|
| `Directory (etc/falcond) is empty`, `Directory (usr/share/falcond/profiles/user) is empty` | intended | The package owns them so the helper's sandbox (`ReadWritePaths=`, `ConfigurationDirectory=`) can write there before falcond is installed; see `package()`. |
| `Dependency cairo / pango / graphene / gdk-pixbuf2 detected and implicitly satisfied` | warning, valid | `bigame-ui` links them directly through gtk4-rs; `gtk4` pulls them in. Arch's guidelines prefer listing direct dependencies; adding them is a packaging decision left to the maintainer. |
| `Dependency included, but may not be needed ('dbus', 'polkit', 'systemd', 'curl', 'libarchive', 'hwdata', 'iputils', 'iproute2')` | false positive | namcap only sees ELF links. These are services the helper runs under (D-Bus, Polkit, systemd), programs Big Game Mode runs (`curl`, `bsdtar`, `ping`, `tc`) and data it reads (`pci.ids`). |

An `E:` line fails the build. A new `W:` line should be read and either fixed
or added here.

### Other decisions

- **Container.** `archlinux:base-devel` is Arch's own image and repositories,
  the ones the BigLinux package builders resolve `depends` against. Each job
  installs only what it uses.
- **No cache.** Arch's `rust` changes with every stable release, which
  invalidates `target/`. A debug `target/` is several GiB, and downloading the
  crates takes seconds. `makepkg --cleanbuild` must start clean anyway. A
  cache would add a failure mode for little gain.
- **No path filters.** Nearly every path ends up in the package (`README.md`
  is installed, `data/`, `locale/`, `style/` and `usr/` are compiled in or
  installed). A filter loose enough to be safe would save almost nothing.
- **Tags and releases.** Nothing is published from CI. The BigLinux builders
  publish packages; the artifact is for testing.
- **Permissions.** `contents: read` at workflow level, no secrets. No
  `pull_request_target`.
- **Third-party actions.** Only `actions/checkout` (v7.0.1) and
  `actions/upload-artifact` (v7.0.1), pinned by commit. The pins were
  checked against the tags' commits. Nothing is downloaded but Arch's signed
  packages and the crates `Cargo.lock` pins by checksum.
- **makepkg runs `check()` in CI.** It is the slowest part of the build
  (fat-LTO test binaries) and repeats the tests `backend-tests` runs in debug
  mode. It stays because it is part of the real recipe: if `check()` fails, the
  BigLinux builders fail too, and only this job would notice.

## Reproducing locally

From the repository root, on BigLinux/Arch with the build dependencies:

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

The package, as CI builds it. This builds committed content only, so commit
first:

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

In a container identical to CI's:

```bash
docker run --rm -it --init -v "$PWD":/src:ro archlinux:base-devel bash
# inside: pacman -Syu --noconfirm git namcap desktop-file-utils appstream libxml2
#         useradd -m builder; echo 'builder ALL=(root) NOPASSWD: /usr/bin/pacman' > /etc/sudoers.d/builder
#         runuser -u builder -- git clone /src /home/builder/bigamemode
#         then the commands above as builder
```

`actionlint .github/workflows/*.yml` (with `shellcheck` installed it also
checks every `run:` block) and `shellcheck .github/scripts/*.sh tests/*.sh`
check the workflows themselves.

## Investigating a failure

1. Open the run's **Summary**. The table shows which check failed and which
   ones did not run.
2. `gh run view <run-id> --repo biglinux/bigamemode --log-failed` shows the
   failing step's log.
3. Download the artifacts:
   - `backend-test-logs-…`: `cargo-test.log`, `daemon-authorization.log` and
     `ci-versions.log` (commit, toolchain and every installed package with its
     version, which matters when Arch moves on);
   - `bigame-mode-build-logs-…` after a failed package build, or
     `bigame-mode-<version>-<commit>` after a successful one: makepkg's log
     per stage (`*-prepare.log`, `*-build.log`, `*-check.log`,
     `*-package.log`), `namcap.log`, `package-info.txt`.
4. A failure right after an Arch update (new rustc, new Clippy lint, new
   namcap rule) shows up as a version change in `ci-versions.log` against
   the last green run.

## Pending

- **`rust-version` is wrong.** `bigame-engine/Cargo.toml` declares 1.85, but
  `cargo +1.85 check --locked --workspace --all-targets` fails: zbus 5.14,
  zvariant 5.10 and their macros need 1.87, and the code uses let chains,
  which need 1.88 (`cargo +1.87 check`: *`let` expressions in this position
  are unstable*). `cargo +1.88 check` passes. Setting it to 1.88 makes
  Clippy's MSRV-aware lints apply. With Clippy 1.99 that is 81 mechanical
  rewrites in 42 files of `bigame-core` and `bigame-ui`: 76 nested `if`s into
  let chains (`collapsible_if`), 3 `chunks_exact` into `as_chunks`, 2
  `% 2 == 0` into `is_multiple_of`. Clippy no longer suggests them because
  1.85 predates those features. This belongs in its own pull request:
  `rust-version = "1.88"` (and the README's *Requires Rust* and badge), the
  rewrites (`cargo clippy --fix`, then review), and a job that keeps the
  declaration honest:

  ```yaml
  msrv:
    name: Minimum supported Rust (rust-version)
    runs-on: ubuntu-24.04
    container: archlinux:base-devel
    timeout-minutes: 20
    defaults:
      run:
        shell: bash
    steps:
      - name: Prepare Arch environment
        run: pacman -Syu --noconfirm --needed git rustup gtk4 libadwaita glib2
      - name: Check out the commit under test
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Build the workspace with the declared minimum Rust
        working-directory: bigame-engine
        run: |
          set -Eeuo pipefail
          msrv=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^rust-version *= *"\([^"]*\)".*/\1/p' Cargo.toml)
          test -n "$msrv"
          # rustup verifies each component against the release manifest's SHA-256.
          rustup toolchain install "$msrv" --profile minimal --no-self-update
          cargo "+$msrv" check --locked --workspace --all-targets --all-features
  ```

  Tested while preparing this change: in a local replay of the job in
  `archlinux:base-devel`, it passed with `rust-version = "1.88"`;
  `cargo +1.85 check` fails as described above.
- **The builders' trigger is seen from outside only.** It was read from the
  builder's public job history, not from its configuration. The first push
  to `main` after this change, a successful Build Package run followed by a
  `main` job on the builder, confirms it.
- **`check()` duration.** About 10 of the package job's ~18 minutes are
  `cargo test --release` linking every test binary with fat LTO
  (`[profile.release]`). Testing without `--release` in `check()` would be
  much faster, but it changes the recipe the BigLinux builders run and is the
  maintainer's call.
- **Desktop entry hint.** `desktop-file-validate` passes, with one hint:
  `Categories=Game;System;Settings;` names three main categories, so some
  menus may list the application more than once.
- **Direct library dependencies.** namcap's *implicitly satisfied* warnings
  (cairo, pango, graphene, gdk-pixbuf2): listing them in `depends` is Arch's
  guideline; see the namcap table.

## Manual / integration testing required

CI runs in a container: no systemd as PID 1, no system bus, no Polkit agent,
no GPU and no games. These need a real BigLinux installation:

- the helper started by D-Bus activation through `bigame-daemon.service`, its
  sandbox (`ProtectSystem=strict`, `ReadWritePaths=`, `CapabilityBoundingSet=`)
  and a Polkit prompt that grants a method;
- falcond taken over and handed back (Turbo, `ReleaseGameBackend`,
  `pre_remove`), `scx_loader` scheduler switches, power-profiles-daemon;
- install, upgrade and removal through pacman (`bigame-mode.install`);
- Gamescope, MangoHud, vkBasalt, lsfg-vk, OptiScaler downloads, Steam, Heroic,
  Lutris and Proton games;
- GPU telemetry (NVML, AMD sysfs), hybrid graphics, 3D V-Cache CPUs;
- the GTK interface and the tray icon on a desktop session.

CI covers what does not need them: the helper refusing every privileged call
without Polkit, the unit, the activation file and the bus policy agreeing with
each other and with the installed binary, and the policy files being
well-formed.
