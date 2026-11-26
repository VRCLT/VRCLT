# Windows releases

The application version comes from `Cargo.toml`. Update the package version and lockfile, add the matching section to `CHANGELOG.md`, and write `docs/releases/v<version>.md`. The first heading is the GitHub release title.

On Windows x64 with Rust stable MSVC, Visual Studio C++ tools, CMake and libclang installed, run:

```powershell
cargo fmt --all --check
./scripts/build_native.ps1 -Test
./scripts/package_release.ps1 -SkipBuild
```

The packager checks the Windows product version, creates the EXE and ZIP in `release/`, compares the archived executable with the standalone build, and writes a SHA-256 file for each download. The ZIP contains `vrclt/vrclt.exe` and the licenses.

Open a pull request into `main`. The required `build` check runs the Rust tests on Windows and attaches both packages. When a new Cargo version reaches `main`, CI builds the merged revision, verifies its checksums, tags the revision, and publishes those files to GitHub Releases. Existing published versions are left intact.

The site workflow builds the five-language download site from `site/` and publishes it to GitHub Pages. Device, headset and paid-provider integration tests are opt-in and are identified by the Rust test harness.
