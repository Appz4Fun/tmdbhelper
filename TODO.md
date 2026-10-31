# To do

## Windows build, beta

`just build-release-win` cross-compiles `x86_64-pc-windows-gnu` with
[cross](https://github.com/cross-rs/cross) and MinGW. Status:

- [ ] Run the binary on Windows. Nobody has tested it there yet.
- [ ] Check the free-space check and the default `./output` paths on Windows.
- [ ] Decide whether to ship an MSVC build, which needs a Windows runner or
      `cargo-xwin`.

## CoreELEC package

Today, installing on a Kodi box means copying the binary and the systemd units
by hand. See [issue 3](https://github.com/Appz4Fun/tmdbhelper/issues/3).

- [ ] Build an opkg package for Entware, because many CoreELEC users already
      have opkg.
- [ ] Consider a full Kodi add-on later, which would install from Kodi's own
      interface.
