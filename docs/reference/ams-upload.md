# AMS Image Upload — Setup and Troubleshooting

How to configure an IAM client for `ags ams upload`, and how to read the errors when it
refuses.

For the normative requirements (flags, validation rules, dry-run semantics) see
[`cli-reference.md`](cli-reference.md) §10.5.5. This page is the practical companion.

## Quick start

```bash
export AGS_CLIENT_ID=<client id>
export AGS_CLIENT_SECRET=<client secret>
ags auth login --grant client-credentials
ags ams upload --path ./build --executable server --image-name my-image
```

## The IAM client

**It must be a Confidential client.** Client credentials require a secret, and only a
confidential client has one — a public client cannot authenticate this way.

**Create it in the namespace the images should land in** — normally your game namespace,
not the publisher namespace. AMS reads the destination from the token's namespace claim,
so the client's own namespace *is* the destination. This is why `--namespace` is accepted
but ignored on this command: there is no way to redirect an upload elsewhere.

That namespace must have AMS enabled. If it does not, the upload fails with
`no account associated with namespace <ns>`.

## Minimum permissions

| To do this | Permission | Actions | Prefix |
|---|---|---|---|
| **Upload an image** (required) | `AMS:UPLOAD` | `Create`, `Update` | **None** — enter it bare |
| Verify with `ags ams images list` / `get` (optional) | `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE` | `Read` | **Namespaced** |

### Watch the prefix

The two permissions look related and are easy to confuse, but the prefix rule differs and
**getting it wrong produces a permission error, not a validation error** — so the mistake
gives no signal about itself.

They are enforced by two different systems:

- `AMS:UPLOAD` is checked by **AMS itself**, on a request that goes straight to the AMS
  host carrying your token. AMS compares the resource string exactly, with no namespace
  prefix.
- `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE` is checked by the **AGS gateway**, which proxies
  the `ags ams images` commands and then talks to AMS using its own service token.

A consequence worth internalising: **being able to list images does not imply being able to
upload one, and vice versa.** Both directions occur in practice — a user with the stock
`AMS Access` role can list but not upload; a client provisioned only for CI can upload but
not list.

### Both actions are required

`Create` covers image creation and URL signing; `Update` covers multipart finalize and
marking the image complete. A `Create`-only identity uploads every byte and *then* fails at
the last step.

This matches AccelByte's
[CI/CD upload guide](https://docs.accelbyte.io/gaming-services/modules/multiplayer/multiplayer-servers/how-to/automate-dedicated-server-uploads-in-cicd/),
which documents the requirement as `AMS:UPLOAD (Create, Update)`.

### The stock `AMS Access` role is not enough

It grants `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE` (full CRUD) plus fleet and account
permissions, and **no** `AMS:UPLOAD`. Holding it lets you manage images but never create
one.

## Interactive use

`ags auth login` (browser / authorization code) works too, but the permission then has to
sit on your **user's roles** rather than on a client. Note that a role assignment does not
reach an existing token: see the refresh row in the table below.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `403 token is missing required permissions` while creating the image | The identity lacks `AMS:UPLOAD` | Grant `AMS:UPLOAD` with `Create` + `Update`, **un-prefixed**, then re-authenticate |
| `403 token is missing required permissions` while finalizing or completing the upload | The identity has `Create` but not `Update` on `AMS:UPLOAD` — everything uploads, then the last call fails | Add the `Update` action to `AMS:UPLOAD`, then re-authenticate |
| Upload succeeds, but `ags ams images list` returns `20013 You do not have permission` | Has `AMS:UPLOAD` but not `AMS:IMAGE` | Add `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE` with `Read` |
| `404 no account associated with namespace <ns>` | The client's namespace has no AMS account | Use a client in a namespace with AMS enabled |
| Permission change appears to have no effect | The token predates the change | `ags auth refresh` after a client-credentials login. After a **browser** login a full `ags auth login` is required — refresh does not recompute role grants |
| `Could not determine the AMS upload host` | Host discovery failed | Check the base URL and connectivity, or pass `--upload-url`. The CLI fails here rather than falling back to production |
| `Executable must be a 64-bit little-endian ELF binary, or a shell script (.sh)` | Wrong architecture or a non-ELF file | Rebuild for `linux-x86_64` or `linux-arm_64` |
| `Target architecture is required when the entrypoint is a shell script` | A `.sh` entrypoint carries nothing to detect | Pass `--target-arch` |
| `does not match the on-disk filename case` | `--executable` case differs from the file | Match it exactly — Linux is case-sensitive even where your machine is not |
| An incomplete image is left in the namespace after a failed upload | The record is created before the bytes are shipped | See [Failed uploads leave an image record](#failed-uploads-leave-an-image-record) |

Run with `--dry-run` to check the build directory, entrypoint, and detected architecture
without authenticating or uploading anything.

### Failed uploads leave an image record

`upload` registers the image with AMS before transferring anything, so any failure after
that point — a dropped part, finalize, or the completion call — leaves an incomplete
record behind. The CLI names it, with its id, on the second line of the error.

Nothing removes it automatically, and the image name is not unique: every upload mints a
new id, so a CI job retrying a flaky upload accumulates one incomplete record per attempt.
They count against image storage (`ags ams images get-storage`).

Removing one needs `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE` with `Delete`, which is a
**different permission from `AMS:UPLOAD`** — an upload-only client cannot clean up after
itself. Either grant `Delete` to the client that uploads, or remove the image from the
Admin Portal:

```bash
ags ams images mark-for-deletion --image-id <image-id> --namespace <namespace>
```

## Migrating from the standalone `ams` CLI (`armada-cli`)

| Old | New |
|---|---|
| `-c` / `-s` | `AGS_CLIENT_ID` / `AGS_CLIENT_SECRET` |
| `-H <host>` | the profile's base URL |
| `--targetArch` | `--target-arch` |
| `--imageName` | `--image-name` |
| `--symbolFiles` | `--symbol-files` |
| `UPLOAD_SERVICE_URL` | `--upload-url` (the env var is inert) |

**Keep using the same IAM client.** `armada-cli` only ever authenticated as a client, so
your existing upload client already holds `AMS:UPLOAD`. Moving those two values to the
environment variables is the whole migration. Switching a pipeline to `ags auth login`
instead will fail with a 403 unless the permission is also granted to a user's roles.

Two behaviour changes to be aware of: an unreachable platform host is now a hard error
rather than a silent fall back to production, and credentials are no longer accepted as
flags, so they stop appearing in shell history and process listings.
