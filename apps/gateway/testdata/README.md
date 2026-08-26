# Test fixtures

`vertex_test_key.pem` is a throwaway RSA private key generated locally with
`openssl genpkey`, used only to unit-test JWT assertion construction in
`providers/vertex.rs`. It is not connected to any Google Cloud project, any real
service account, or any credential that has ever existed outside this repository.
Committing it is safe and intentional — the whole point is that CI can sign a test
assertion without any real secret.
