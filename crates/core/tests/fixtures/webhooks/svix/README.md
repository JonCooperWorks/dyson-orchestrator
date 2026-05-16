Svix fixture for Dyson Swarm verifier tests.

Svix documents webhook verification:

- https://docs.svix.com/receiving/verifying-payloads/how
- https://docs.svix.com/receiving/verifying-payloads/how-manual

Headers:

`svix-id: msg_svix_1`
`svix-timestamp: 1700000000`
`svix-signature: v1,rIQVOgymL66XhZlMhPJ2Ib+z0VNjxIz05sLiJMjKXhU=`

The signed payload is `msg_svix_1.1700000000.<raw body>`. The
secret uses the Svix `whsec_` form; the verifier decodes the base64
portion after the prefix and uses those bytes as the HMAC key.
