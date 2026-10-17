# Conduit

The [RealWorld](https://realworld-docs.netlify.app) backend: users with JWT
authentication, profiles and follows, articles with tags, a personal feed,
favorites, comments, and CORS. Everything comes from Rynd's standard
builtins (`serve`, `jwt_sign`, `password_hash`, `store_*`) with no crates.

```sh
PORT=3000 CONDUIT_DB=conduit.db JWT_SECRET=change-me rynd run examples/conduit/main.rynd
rynd build examples/conduit/main.rynd -o conduit   # the same app as a native binary
```

Point any RealWorld frontend at `http://127.0.0.1:3000/api`.

| File | Responsibility |
| --- | --- |
| `main.rynd` | Configuration from the environment, the store, and `serve`. |
| `app.rynd` | CORS, authentication, and the route table. |
| `users.rynd` | Registration, login, the current user, profiles, follows. |
| `articles.rynd` | Articles, filters and paging, the feed, favorites, comments, tags. |
| `http.rynd` | JSON responses, RealWorld error bodies, request validation. |

Data lives in one append-only store file (`CONDUIT_DB`, default
`conduit.db`). Without `JWT_SECRET`, a random secret is generated per run, so
tokens stop working after a restart. `tests/conduit_test.rs` runs the API
flows over HTTP against both `rynd run` and the native build, including a
restart and a stalled client that must not block other requests.
