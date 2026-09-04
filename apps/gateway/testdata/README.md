# Test fixtures

`vertex_test_key.pem` is a throwaway RSA private key generated locally with
`openssl genpkey`, used only to unit-test JWT assertion construction in
`providers/vertex.rs`. It is not connected to any Google Cloud project, any real
service account, or any credential that has ever existed outside this repository.
Committing it is safe and intentional — the whole point is that CI can sign a test
assertion without any real secret.

`sso_test_key_a_private.pem` / `sso_test_key_a_public.pem` and the `_b_` pair are two
more throwaway 2048-bit RSA keypairs, generated locally with `openssl genrsa` /
`openssl rsa -pubout`, used only to unit-test id-token signature verification in
`enterprise::sso`. Same guarantee as the Vertex key: not connected to any real account,
service, or credential. Two distinct keypairs exist specifically so a test can prove a
token signed by key A is rejected when verified against key B's public key — the actual
signature check, not just claim-shape validation.

These replaced an earlier draft that generated fresh keys at test time via the `rsa`
crate as a dev-dependency. `cargo audit` (which CI runs with `--deny warnings`) flags
that crate under RUSTSEC-2023-0071, a timing side-channel with no fixed release
available. `jsonwebtoken` itself signs and verifies through `ring`, not `rsa`, so static
fixtures give identical real-RS256 test coverage with no vulnerable dependency at all.
