# gclba-index

Private GCLBA property intelligence index.

This service ingests approved property exports, normalizes parcel and address
keys, assembles per-parcel dossiers, and emits the conflict ledger that Django
imports into the GCLBA portal.

## Boundary

- Canonical system of record: Django/PostGIS in `compliance-inspection-tracker`.
- Map surface: `our-civic-atlas-gclba`.
- This repo: private ingestion, resolution, and read-model export.
- PorchFest is not part of this checkout, build, or deployment.

PII is dropped at the adapter boundary. Buyer names, emails, phone numbers,
organizations, notes, and reviewed-by fields are not hashed or retained.

## Normalization Contract

The adapter pre-pass uses pure normalization functions before creating index
records:

- Parcel IDs strip non-digits and accept exactly 10 digits, then render as
  `XX-XX-XXX-XXX`. Anything else is unresolved and must flow to conflict
  handling instead of being padded.
- Address keys uppercase, remove punctuation, standardize directionals and USPS
  suffixes, and drop unit designators plus everything after them.
- Owner names uppercase, remove punctuation, and collapse whitespace without
  replacing the original owner identity. GCLBA labeling uses the fixed name set
  in `src/normalize.rs`.
- Condition regression uses the fixed ordering in `CONDITION_GRADE_ORDER`.

## Datawave Pin

The service validates against a pinned Theorem submodule:

```bash
vendor/theorem/rustyredcore_THG/crates/rustyred-thg-datawave
```

Pinned revision:

```text
e49952a3d899594456adf96c65f98371484133f0
```

That commit is available from `origin/feat/phase-4-reconstruction-backend` in
`https://github.com/Travis-Gilbert/Theorem.git`. Cargo uses a path dependency
into the submodule because the Datawave crate is nested below the Git repository
root.

## CLI

Build a Django-importable JSON payload from one export:

```bash
cargo run -- ingest \
  --adapter site_control_export \
  --file path/to/site-control.csv \
  --output out/index.json
```

Build a combined ledger from multiple exports:

```bash
cargo run -- build \
  --source site_control_export:path/to/site-control.csv \
  --source county_arcgis:path/to/county.csv \
  --source filemaker_csv:path/to/filemaker.csv \
  --output out/index.json
```

Run the internal API from an exported payload:

```bash
cargo run -- serve --index out/index.json --bind 0.0.0.0:8080
```

Run the internal API from uploaded source exports:

```bash
cargo run -- serve --exports-dir exports --bind 0.0.0.0:8080
```

Endpoints:

- `POST /ingest/{adapter}` stores the raw export under
  `exports/{adapter}/` and rebuilds the combined index from every saved export.
- `GET /dossier/{parcel_id}`
- `GET /conflicts?kind=owner_mismatch`
- `GET /coverage`
- `GET /healthz`

No auth gate is wired in this slice because the current deployment target is
fully internal.

## Validation

```bash
cargo fmt --check
cargo test
```

## Railway Deployment Note

Current production service:

- URL: `https://gclba-index-production.up.railway.app`
- Project ID: `5bf96e20-3223-4548-b035-e6405af403f7`
- Service ID: `1fb2906b-257f-47cb-b9c9-1deca82401c9`
- Deployment ID: `cc351080-fa9d-4a10-98f7-9d2607613d7b`

The repository includes the Theorem engine as a private Git submodule. Railway's
standard GitHub autodeploy requires a connected GitHub account with contributor
access to the repository, and Railway support notes that private submodule
authentication may require a Dockerfile-based deployment path. Keep deployment
secrets in Railway variables, never in this repo.
