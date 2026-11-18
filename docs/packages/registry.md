# Registries and hosting

A Vard registry is **static files**. Reading is plain `GET`s with no server logic; writing
(publishing) happens in one place and always passes the same gate. So a registry can live
anywhere — in a directory, in an S3/R2/GCS bucket behind a CDN, on GitHub Pages, on nginx —
and be served through untrusted mirrors.

## The public registry

[`vardemin/vard-registry`](https://github.com/vardemin/vard-registry) is the default
registry of `vard` (name `vard`, `https://vardemin.github.io/vard-registry`). It is this
same format, kept in a GitHub repository and served by GitHub Pages — free for open source,
no server to run:

- **Publishing is a pull request.** Copy the package to `submissions/<name>/` and open a
  PR; the `check` workflow runs `vard publish --dry-run` with the PR author as the owner.
  After a merge, the `publish` workflow publishes as the PR author, removes the submission,
  commits `public/` and redeploys Pages.
- **The signing key** lives only in the repository's Actions secrets (and with the
  maintainer); its public half is **compiled into `vard`**, so even the first contact is
  verified — no trust-on-first-use window.
- **Ownership** is the GitHub login of the first publisher; later versions must come from
  their pull requests.

The rest of this page describes the format and how to run a registry of your own.

## Create a registry

```bash
vard registry init ./reg --name acme
vard registry token ./reg alice            # a publish token, shown once
vard publish ./charts --registry ./reg --token vard_…
```

```text
reg/
  public/                 ← this is the registry: serve it as static files
    config.json           {"vard_registry": 1, "name": "acme", "key": "<ed25519 public key>"}
    catalog.json          one row per package: name latest summary keywords caps exports tests
    index/charts.json     JSON lines: one per version (+ yank status lines)
    blobs/<sha256>.vpk    immutable archives
    log.txt               the publication log (hash chain)
    log.head              size, chain head, ed25519 signature
    index.html            the registry's web page: search, versions, capabilities, API, provenance
  private/                ← never publish this
    registry.key          the signing key
    tokens                sha256(token) owner
```

## Read protocol

| Request | Response | Caching |
|---|---|---|
| `GET config.json` | name, protocol version, public key | short |
| `GET catalog.json` | the catalog for search (search runs on the client — the server computes nothing) | short |
| `GET index/NAME.json` | every version of a package; 404 — no such package | short |
| `GET blobs/SHA256.vpk` | an archive | forever (`immutable`): the address is the content hash |
| `GET log.txt`, `GET log.head` | the signed log | short |

An index line:

```json
{"name":"charts","version":"1.0.0","sha256":"100e…","size":822,"vard":">=0.1.0",
 "deps":[{"name":"tailwind","req":"1.0","registry":"builtin"}],
 "caps":[],"android":null,"summary":"Bar charts drawn with Vard elements","keywords":["chart"],
 "api":[{"kind":"view","name":"BarChart","params":[{"name":"values","ty":"[Int]","default":null,"bind":false}]}],
 "doc_bytes":78,"tests":{"passed":1,"failed":0},"tested_with":"0.3.0","owner":"alice","yanked":false,
 "published":"2026-11-19T08:30:00Z",
 "provenance":{"builder":"github-actions","repository":"https://github.com/vardemin/vard-registry","commit":"…","run":"…/actions/runs/…"}}
```

`api` is the full public API computed by the compiler at publish: `vard info`, search by
exports and the API diff in `vard update` work from it without downloading archives. A
dependency's `registry` is where it resolved for the author (`builtin`, a URL or a
directory), so the consumer gets it from the same place.

Yanking is an appended line `{"name":"charts","version":"1.0.1","yanked":true}`: the index is
append-only.

## The log and its signature

```text
log.txt    1 charts 1.0.0 <archive sha256> <chain_1>
           2 charts 2.0.0 <archive sha256> <chain_2>
           chain_n = sha256(chain_{n-1} " " n " " name " " version " " sha)
log.head   2 <chain_2> <ed25519("vard-log/1 2 <chain_2>")>
```

On first contact with a registry the client:

1. pins its key (`~/.vard/known_registries`, like SSH's `known_hosts`); a changed key is `P005`;
2. verifies the head's signature and recomputes the chain;
3. checks that the head it saw last time is a prefix of the new log (history was not
   rewritten) — `P005`;
4. installs only an archive whose `(name, version, sha256)` is in the log and whose hash
   matches — `P004`.

`vard registry verify DIR|URL` verifies a whole registry: signatures, chain and every archive.

### Rotating the key

```bash
vard registry rotate-key ./reg
```

The old key signs a rotation record `{from, to, sig}` that stays in `config.json`; the log
head is re-signed with the new key, and the old secret is kept as
`private/registry.key.<id>.old`. A client that trusts the old key (pinned, or compiled in for
the public registry) follows the chain of signed rotations to the current key and re-pins it
silently; a key that no signed rotation links to is still `P005`. After rotating, publish
`public/` and update the CI secret with the new `private/registry.key`.

## The web page

Every registry gets `public/index.html` (written by `vard registry init`, added by the next
publish to older registries): a single static page, no external scripts, that reads
`catalog.json` and `index/NAME.json`. It shows search over names, keywords, summaries and
exports; capabilities with sensitive ones highlighted; publish-time test results; versions
with publication time, yank status and provenance; the generated API; and the
`vard add NAME` command. `vard registry serve` serves it at `/`.

## Hosting

### A directory or a network share

`[registries] acme = "/mnt/share/vard-registry"` — the client reads the files directly. Good
for a team and for CI.

### `vard registry serve`

```bash
vard registry serve ./reg --addr 0.0.0.0:8470
```

- `GET /…` — static files from `public/`; archives with `Cache-Control: immutable`.
- `POST /api/v1/publish` — body: a `.vpk`, header `Authorization: Bearer TOKEN`. The server
  unpacks the package into a scratch directory, runs **the same gate** as `vard publish`
  (compile, tests, semver, capabilities…) and appends to the log. The response is the gate
  report.
- `POST /api/v1/yank` — body `NAME VERSION [undo]`, owner only.

The first publisher of a name becomes its owner; other tokens get `P018`. The server speaks
HTTP — put it behind any TLS terminator. Integrity does not depend on TLS.

### A CDN or a bucket

Publish locally (`vard publish --registry ./reg`) or in CI, then sync `public/`:

```bash
aws s3 sync reg/public s3://vard-registry --cache-control "no-cache"
aws s3 cp reg/public/blobs s3://vard-registry/blobs --recursive --cache-control "public, max-age=31536000, immutable"
```

Clients: `[registries] acme = "https://registry.acme.dev"`.

### A registry in git (GitOps)

Keep `public/` in a git repository. Publishing = a pull request that CI checks with
`vard publish --dry-run` and, after merge, applies with `vard publish --registry` using the
key from CI secrets; GitHub Pages serves the result. The registry's history gets a second,
independent log — git.

## Client configuration

| Where | What |
|---|---|
| `vard.toml` `[registries]` | the project's registries and `default` |
| `~/.vard/config.toml` `[registries]` | the user's registries (`default = "acme"`) |
| `VARD_REGISTRY` | the default registry for every project (CI) |
| `~/.vard/credentials.toml` `[tokens] acme = "vard_…"` or `VARD_TOKEN` | publish tokens |
| `~/.vard/known_registries` | pinned keys |
| `~/.vard/cache/` | archives (`blobs/`), unpacked packages (`pkgs/`), seen log heads |

The `builtin` registry is the packages from the framework repository's `packages/`, compiled
into the binary. It works offline and needs no signature: its content is part of the
executable.

## Private registries and mixing

A dependency comes from exactly one registry: its own `registry = "..."` or the default
registry. A name from a private registry cannot be "hijacked" by a public one, and vice
versa. For mirrors: a mirror needs no trust — it serves the same signed files.

## Scaling

The current format targets thousands of packages: a client reads only the indexes of the
packages it needs; the catalog is one file. For large registries (roadmap): a Merkle tree
instead of the full log (O(log n) inclusion proofs, like Go's sumdb), a sharded catalog, key
rotation with the new key signed by the old one.
