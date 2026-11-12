# zegadb

`zega`, the command line for [Zega](https://zega.dev): run a Zega database locally, open the explorer, and build and deploy with Zega Cloud.

```sh
npm install --save-dev zegadb
npx zega start          # a local Zega database (Zega+SQLite)
npx zega --help
```

Or without npm: `curl -fsSL https://zega.dev/install.sh | sh`.

This package is a small Node launcher. npm installs the executables for your platform from `@zegadb/server-<os>-<cpu>` (macOS arm64 and x64, Linux x64), an optional dependency at the same version, so don't install with `--omit=optional`. No Bun, no download, no install script.

Source: https://github.com/zegadb/zega
