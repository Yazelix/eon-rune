# Check inputs

`checks.sh` owns the required phase order and invokes the compiler, focused
regressions, isolated primary/idle/resize checks and no-fade motion gate.
`required-tests.json` names passing regressions, rather than treating a test
count as proof. The frozen motion fixture, scenario and witness checker are
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
not Eon delivery or its test suite. All raw measurements and session records
belong outside the checkout.
