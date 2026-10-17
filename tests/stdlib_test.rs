//! Hashing, tokens, passwords, time, slugs, and the document store.
use rynd::{RyndEngine, Value};

fn eval(code: &str) -> String {
    RyndEngine::new()
        .eval(code)
        .unwrap_or_else(|e| panic!("{e}\n{code}"))
        .to_string()
}

#[test]
fn hashes_and_encodings_match_reference_vectors() {
    assert_eq!(
        eval(r#"[sha256("abc"), hmac_sha256("Jefe", "what do ya want for nothing?")]"#),
        "[ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad, 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843]"
    );
    assert_eq!(
        eval(
            r#"[base64_encode("foob"), base64url_encode([251, 255]), base64_decode("Zm9vYg=="),
                base64_decode_bytes("-_8"), hex_encode([0, 255]), to_bytes("é"), from_bytes([104, 105])]"#
        ),
        "[Zm9vYg==, -_8, foob, [251, 255], 00ff, [195, 169], hi]"
    );
    for bad in [
        r#"base64_decode("!!")"#,
        r#"from_bytes([255])"#,
        r#"sha256(3)"#,
    ] {
        assert!(RyndEngine::new().eval(bad).is_err(), "{bad}");
    }
}

#[test]
fn tokens_verify_and_expire() {
    assert_eq!(
        eval(
            r#"
            let token = jwt_sign({"sub": 7, "exp": now() / 1000 + 60}, "secret")
            let expired = jwt_sign({"sub": 7, "exp": now() / 1000 - 1}, "secret")
            let parts = split(token, ".")
            let forged = parts[0] + "." + base64url_encode("{\"sub\":1}") + "." + parts[2]
            [jwt_verify(token, "secret").value.sub, jwt_verify(token, "other"),
             jwt_verify(expired, "secret"), jwt_verify(forged, "secret"), jwt_verify("x.y", "secret")]
        "#
        ),
        "[7, Err(invalid signature), Err(token expired), Err(invalid signature), Err(malformed token)]"
    );
}

#[test]
fn passwords_and_randomness() {
    assert_eq!(
        eval(
            r#"
            let stored = password_hash("correct horse")
            [starts_with(stored, "$pbkdf2-sha256$100000$"), password_verify("correct horse", stored),
             password_verify("wrong", stored), stored == password_hash("correct horse"),
             secure_compare("abc", "abc"), secure_compare("abc", "abd")]
        "#
        ),
        "[true, true, false, false, true, false]"
    );
    assert_eq!(
        eval(
            r#"
            let ids = range(0, 50) |> map(\_ -> uuid())
            let ints = range(0, 500) |> map(\_ -> random_int(-2, 3))
            [len(uniq(ids)), len(ids[0]), ids[0][14], all(ints, \n -> n >= -2 and n < 3),
             len(uniq(ints)), len(random_bytes(16)), len(random_token(32))]
        "#
        ),
        "[50, 36, 4, true, 5, 16, 43]"
    );
}

#[test]
fn times_format_and_parse_as_rfc3339() {
    assert_eq!(
        eval(
            r#"
            [iso_time(0), iso_time(951782400123), iso_time(-1),
             parse_time("2000-02-29T00:00:00.123Z"), parse_time("1970-01-01T01:00:00+01:00"),
             parse_time("2026-10-17"), parse_time(iso_time(1760693000456)),
             attempt(parse_time, ["2026-02-30"]).tag, attempt(parse_time, ["yesterday"]).tag,
             now() > 1700000000000]
        "#
        ),
        "[1970-01-01T00:00:00.000Z, 2000-02-29T00:00:00.123Z, 1969-12-31T23:59:59.999Z, 951782400123, 0, 1792195200000, 1760693000456, Err, Err, true]"
    );
    assert_eq!(
        eval(
            r#"[slugify("How to Train Your Dragon!"), slugify("  --Rynd 2.0 -- "), slugify("☃")]"#
        ),
        "[how-to-train-your-dragon, rynd-2-0, ]"
    );
}

fn scratch(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("stores");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

#[test]
fn stores_persist_query_and_survive_torn_writes() {
    let path = scratch("people.db");
    let mut engine = RyndEngine::new();
    engine.set_global("path", Value::string(path.clone()));
    assert_eq!(
        engine
            .eval(
                r#"
                let db = store_open(path)
                store_put(db, "people", store_next_id(db, "people"), {"name": "Ada", "lang": "rynd"})
                store_put(db, "people", store_next_id(db, "people"), {"name": "Grace", "lang": "cobol"})
                store_put(db, "people", 10, {"name": "Linus", "lang": "rynd"})
                store_put(db, "people", "x", {"name": "Key", "lang": "str"})
                store_delete(db, "people", 2)
                store_close(db)
                "ok"
            "#
            )
            .unwrap()
            .to_string(),
        "ok"
    );
    // A crash mid-append leaves a partial last line; reopening skips it.
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, br#"{"put":["people",99,{"na"#))
        .unwrap();
    let reopened = engine
        .eval(
            r#"
            let db = store_open(path)
            [store_all(db, "people") |> map(\p -> p.name), store_count(db, "people"),
             store_find(db, "people", {"lang": "rynd"}) |> map(\p -> p.name),
             store_get(db, "people", 1).name, store_get(db, "people", 2), store_get(db, "nothing", 1),
             store_next_id(db, "people"), store_delete(db, "people", 404)]
        "#,
        )
        .unwrap();
    assert_eq!(
        reopened.to_string(),
        "[[Ada, Linus, Key], 3, [Ada, Linus], Ada, nil, nil, 3, false]"
    );
}

#[test]
fn stores_compact_their_logs_and_stay_with_their_thread() {
    let path = scratch("counter.db");
    let mut engine = RyndEngine::new();
    engine.set_global("path", Value::string(path.clone()));
    engine
        .eval(
            r#"
            let db = store_open(path)
            range(0, 3000) |> each(\i -> store_put(db, "counter", "only", {"n": i}))
        "#,
        )
        .unwrap();
    let lines = std::fs::read_to_string(&path).unwrap().lines().count();
    assert!(lines < 2000, "log was never compacted: {lines} lines");
    assert_eq!(
        engine
            .eval(r#"store_get(store_open(path), "counter", "only").n"#)
            .unwrap(),
        Value::Int(2999)
    );
    // Store values are not shared with engines running on other threads.
    let id = engine.eval("db.value").unwrap();
    let Value::Int(id) = id else {
        panic!("store handle {id}")
    };
    let error = std::thread::spawn(move || {
        let mut other = RyndEngine::new();
        other.set_global("db", Value::variant("Store", vec![Value::Int(id)]));
        other
            .eval(r#"store_get(db, "counter", "only")"#)
            .unwrap_err()
            .to_string()
    })
    .join()
    .unwrap();
    assert!(error.contains("another program thread"), "{error}");
}
