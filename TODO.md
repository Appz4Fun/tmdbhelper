# To do

## Windows build, beta

`just build-release-win` cross-compiles `x86_64-pc-windows-gnu` with
[cross](https://github.com/cross-rs/cross) and MinGW. Status:

- [ ] Run the binary on Windows. Nobody has tested it there yet.
- [ ] Check the free-space check and the default `./output` paths on Windows.
- [ ] Decide whether to ship an MSVC build, which needs a Windows runner or
      `cargo-xwin`.

## Docs site

- [ ] Turn on GitHub Pages for the repository, with **GitHub Actions** as the
      source, so the `docs` workflow can deploy.
