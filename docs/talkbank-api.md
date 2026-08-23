# Notes on the TalkBank service

Everything here was measured against the live service and is pinned by the
network test suite (`cargo test -p talkbank-archive --test network -- --ignored`).
It is written down because none of it is documented anywhere else, and most of it
is counter-intuitive.

## Access

- The catalogue (`getAnnoPathTrees`, 4.3 MB) and the metadata are **public**;
  downloading requires an account.
- The password field in the login body is **`pswd`**, not `password`. With
  `password` the server answers `NOT_MATCHED` even for correct credentials.
- Authentication is by cookie (`talkbank`, domain `.talkbank.org`, 24 hours), so
  one session covers both the JSON calls on `sla2.talkbank.org` and the downloads
  on `talkbank.org`.
- The access gate answers **200 with `text/html`**, not 401. To tell whether a
  zip actually arrived you have to look at the content type and the `PK`
  signature.
- Restricted banks answer **401**, which is a different case from the access gate
  and has to be reported differently.

## Routes

- `getTranscriptSummary` and `getParticipantSummary` answer
  `{authStatus, colHeadings, data}` **with no `respMsg`** — a generic helper that
  unwraps `respMsg` breaks on two routes out of three.
- Rows in `data` must be resolved by name through `colHeadings`, never indexed by
  position.
- Five routes documented by the official clients (`getNgrams`, `getTokenSummary`,
  `getUtteranceSummary`, `getStats`, `cql`) answer 404 today. A route that
  disappears degrades a feature; it does not fail the app.
- Latency is unpredictable: `getParticipantSummary` took 1.3 s on one corpus and
  35.8 s on another.

## The shape of the tree

- **Depth is not uniform**: 15 banks, 1,897 corpus folders, 97,052 transcripts,
  with leaves between the second and the eighth level.
- Neither depth nor the presence of direct files tells you which folder is a
  corpus. `childes/Eng-NA/Brown` downloads, `childes/Eng-NA` does not, and both
  hold only subfolders. A HEAD request decides.
- The level a corpus sits at **is not fixed even within one bank**: `ca/ATC` is
  downloadable at the second level, `childes/Eng-NA/Brown` at the third, and
  `childes/Clinical-Eng/Conti` is not downloadable while its children `Conti1` and
  `Conti2` are. A rule based on depth would skip whole corpora, which is why we
  probe.
- That probe **only works with an open session**: without one the access gate
  answers `200 text/html` for any path and would say yes to everything.
- **Only a 404 means "not a corpus"** (it arrives as `text/plain`, nine bytes).
  Treating a 503 as "no" would silently drop a corpus from the plan *and* descend
  into its children, multiplying requests exactly while the server is struggling.
  Other non-2xx statuses are "unverifiable": they are retried once, and three in a
  row stop the walk.
- **Authorisation propagates to descendants**: `aphasia/English/Protocol` and
  `.../Protocol/Adler` both answer 401, so we do not descend below a 401.

## Downloads

- The server sends neither `Content-Length` nor `Accept-Ranges`: progress is in
  bytes with no percentage, and resuming an interrupted transfer is not possible.
- **A corpus zip contains its whole subtree**, nested several levels deep too
  (verified on `Brown` → `Adam/`, `Eve/`, `Sarah/` and on `Demetras2` →
  `Jimmy/father/`, compared exactly against the public tree). That is what lets
  "Download all" stop at the first corpus instead of descending.
- **Media are not in the zip**: `McMillan` declares video on all three of its
  transcripts and its zip weighs 10 KB. Measured at ~23 KB per transcript over
  four corpora, which is the basis of the estimate shown before downloading a
  branch.
- **A UI problem must not stop a transfer**: the progress callback returns `false`
  only on an explicit cancellation. Tying it to the state of the progress channel
  aborted a download in silence when the page showing it was closed.
- **The progress channel closes before the result arrives**, so in `net.rs` the
  two are read in sequence rather than raced: awaiting them with a `select!` lost
  the outcome roughly one time in seven (measured) — a download that finished
  without saying so, and a planning run that never reached its confirmation.
