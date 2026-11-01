<h1 align="center">
  <img src="extra/images/logo.png" width=200 height=200/><br>
  GoldenIDE
</h1>

<h4 align="center">Lightning-fast And Powerful Code Editor</h4>

GoldenIDE is written in pure Rust, with a UI in [Floem](https://github.com/lapce/floem). It is designed with [Rope Science](https://xi-editor.io/docs/rope_science_00.html) from the [Xi-Editor](https://github.com/xi-editor/xi-editor), enabling lightning-fast computation, and leverages [wgpu](https://github.com/gfx-rs/wgpu) for rendering.

GoldenIDE is a fork of [Lapce](https://github.com/lapce/lapce).

## Features

* Built-in LSP ([Language Server Protocol](https://microsoft.github.io/language-server-protocol/)) support to give you intelligent code features such as: completion, diagnostics and code actions
* Modal editing support as first class citizen (Vim-like, and toggleable)
* Built-in remote development support. The remote `goldenide-proxy` binary must be installed manually on the remote host
* Plugins are loaded from the local plugins directory only (there is no plugin marketplace)
* Built-in terminal, so you can execute commands in your workspace, without leaving GoldenIDE
* Built-in coding agent panel (ACP)

## Building

The executable is called `goldend-ide`. See the [build guide](docs/building-from-source.md) to compile from source.

```sh
cargo build --release
./target/release/goldend-ide
```

## Contributing

Guidelines for contributing can be found in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

GoldenIDE is released under the Apache License Version 2. You can find a copy of the license text here: [`LICENSE`](LICENSE). It is derived from Lapce, copyright of its authors.
