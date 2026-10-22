# fluxpeer-desktop

The **fluxpeer** desktop client — a pure-Rust [iced](https://iced.rs) GUI for
managing your fluxpeer mesh networks from the menu bar / system tray.

fluxpeer is open-source, self-hostable mesh networking: a WireGuard-compatible data
plane that prefers direct peer-to-peer paths and falls back to a relay when it can't.
It is split-tunnel by default (you choose what is routed via AllowedIPs) and never
routes `0.0.0.0/0` by default.

## How it works

The desktop app is a thin GUI. It does **not** run the data plane itself; it drives the
local **`fluxpeer up`** daemon over its control API on `127.0.0.1:41999`. You must have
the `fluxpeer` binary installed and the daemon running (the app can start it for you).

Authentication to the local daemon uses a bearer token supplied via the
`FLUXPEER_DAEMON_TOKEN` environment variable (the same token the daemon writes/expects).

The **Kill switch** setting protects Linux clients only. macOS and Windows client
kill-switch are not supported in this release; an active request returns an explicit error.
The macOS **Force this device to act as an exit locally** setting enables provider
NAT through an independent PF anchor; it does not protect this Mac's client
traffic when an exit fails.

On Windows, daemon down/shutdown requests member cleanup through a private pipe
and waits for the process to finish. Timeout or cleanup failure keeps Quit in the
failure dialog; forced termination can leave DNS/NAT state even after route
recovery. The explicit quit override closes the GUI without claiming cleanup
succeeded. Windows runtime validation remains pending; cross-compilation alone
does not verify the driver, route table or service installation.

### Environment variables

| Variable | Purpose |
|---|---|
| `FLUXPEER_DAEMON_TOKEN` | Bearer token for the local `fluxpeer up` daemon control API. **Required.** |
| `FLUXPEER_LANG` | Force the UI language (`en` or `zh`). Defaults to the system locale. |

## Build

This crate is **not** part of the core fluxpeer workspace (different license, heavier
GUI dependencies). Build it on its own:

```bash
cargo build --release
```

The binary is `fluxpeer-desktop` (lowercase, to avoid colliding with the `fluxpeer`
CLI on case-insensitive filesystems). The user-facing name is "Fluxpeer".

> **Note:** `Cargo.lock` is gitignored until the core repository is public, because the
> `fluxpeer-sdk` dependency is a git dependency on that repo. For local development against
> a sibling checkout, point the dep at it via `.cargo/config.toml` (also gitignored):
>
> ```toml
> [patch."https://github.com/fluxpeer/fluxpeer"]
> fluxpeer-sdk = { path = "../fluxpeer/sdk/rust" }
> ```

### Generating app icons

The `icongen` example rasterizes the brand mark (`assets/mark.svg`) into platform icon
sets, reusing the app's own resvg dependency:

```bash
cargo run --release --example icongen -- assets/mark.svg out/Fluxpeer.iconset
```

## License

AGPL-3.0-or-later. See [`LICENSE`](LICENSE) and [`LICENSE-AGPL-3.0.txt`](LICENSE-AGPL-3.0.txt).

A commercial license (to use this client without AGPL obligations) is available:
licensing@fluxpeer.org.

**App Store / marketplaces:** an additional permission lets this AGPL app be
distributed via the Mac App Store / Microsoft Store despite their terms of service —
see [`LICENSE-APP-STORE-EXCEPTION.md`](LICENSE-APP-STORE-EXCEPTION.md).

## Contributing

Contributions are welcome. fluxpeer uses a **Contributor License Agreement** so the
project can ship to app stores and offer a commercial edition — see [`CLA.md`](CLA.md).
Sign off commits with `git commit -s` (DCO) and confirm CLA agreement in your PR.
