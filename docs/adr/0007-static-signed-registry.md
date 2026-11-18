# ADR-0007: A static, content-addressed registry with a signed log

**Status:** accepted

**Context.** A registry must be cheap to host (for a team, a company, a community), work
behind any CDN and through mirrors, and trust in packages must not depend on who serves
them. Server-side read logic (search, version resolution) complicates mirroring and scaling.

**Decision.**
1. The read protocol is static files: `config.json`, `catalog.json` (the client searches),
   `index/NAME.json` (JSON lines, append-only), `blobs/SHA256.vpk` (immutable, cached
   forever).
2. The `.vpk` archive is deterministic (sorted paths, no metadata), so the hash is
   reproducible.
3. The publication log is a hash chain `log.txt` whose head is signed with ed25519
   (`log.head`). A client pins the key on first contact and checks the signature, the chain,
   that the previously seen head is a prefix, and that every installed version is in the log.
4. Writes happen in one place and through one gate: `vard publish --registry DIR`, CI over a
   git repository holding the registry, or `vard registry serve` (an HTTP publish API with
   tokens).
5. HTTPS goes through the system `curl`, so the binary carries no TLS stack; integrity does
   not depend on the transport.

**Consequences.** + a registry is a directory served by anything; mirrors need no trust;
substitution of an archive, the index or the history is detected by the client; one gate for
every publishing path. − the client downloads the full log (for large registries, Merkle
trees and inclusion proofs are on the roadmap); a mirror withholding updates is detectable
only by comparing with another source.
