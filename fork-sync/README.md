# Check inputs

`checks.sh` owns the required phase order and invokes the compiler, focused
regressions, isolated primary/idle/resize checks and no-fade motion gate.
`required-tests.json` names passing regressions, rather than treating a test
count as proof. The tooling scripts, fixtures, frozen motion scenario and witness checker are
hashed in `inputs.sha256`; paths/build outputs are separate from their semantics.

These check helpers and fixture adaptations are Yazelix-owned Apache-2.0 code;
see LICENSE-APACHE. `native/virtual-*-protocol.c` and matching headers retain
their original permissive protocol-generation notices. Rune remains MIT.
The tooling is maintained separately from the runtime patches.
The retained `fixtures/venus/examples/` spike examples and native drivers are
fork-owned sources, first versioned with this tooling; they have no external
source revision. `fixture-source.json` distinguishes them from fetched sources
and records the generated bindings' XML package versions, hashes and licences.

The larger owned fixture source is pinned, not vendored: fixture-source.json
names an immutable Eon revision and SHA-256 for each of 25 input files. Only
the hosted-renderer adaptation is carried as a patch. Compiler, registry
dependencies and winit remain external pinned tools/dependencies. No compiler,
driver, dependency tree or cached binary is committed here.

The fixture source has unit-test sections inherited from that pinned source;
they are not run by these example builds. This tooling proves fork behavior,
not Eon delivery or its test suite. The full and focused Rune build copies
select the reviewed owned winit in their root manifests, keeping that explicit
manifest/lock exception separate from registration-only Rust changes. Native
receipts must name the actual resolved winit revision. All raw measurements and session records
belong outside the checkout.

Run `python3 fork-sync/test-gates.py EXISTING_PRIVATE_PROOF_PARENT` for the
missing-regression/final-verdict and frozen-source tamper controls. These
synthetic controls do not qualify a real candidate or native behavior.

Every heavy call through `launch.py` takes the shared Rune lock before queueing.
`--build` also marks the resulting scope as eligible to run Cargo.
It sets a private allowlisted environment; the bounded runner reuses the
scheduler's 8 GiB/no-swap scope (or creates the equivalent fallback). Resource
violations persist a stop marker and prohibit retries. Native cleanup signals
only started process groups; final cleanup is confined to the owned scope.

`RUNE_SYNC_CARGO_SEED` names approved read-only fetched source data on the build
disk. Only registry/Git data is copied to the private Cargo home; configuration
and credentials are excluded, relative cache links and Git object-store paths
are relocated inside that copy. Every qualification Cargo command is offline,
and builds/tests stay locked. Missing inputs stop; fetch only as a separately
approved preparation step. `test-runtime.py` contains the corresponding negative
controls and runs through `launch.py --build`. The check adapter accepts the
sealed tree ID as its sixth argument; source identification is read-only.

`RUNE_SYNC_LOCKS` names the reviewed private lock record documented in
`FORK-SYNC.md`. Qualification installs its hashed workspace and fixture locks
and uses locked metadata throughout. Candidate and upstream fixture locks may
have different local edges; their external package identities must match.
