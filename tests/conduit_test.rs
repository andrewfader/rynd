//! The RealWorld "Conduit" backend in examples/conduit, exercised over real
//! HTTP against both the bytecode VM (`rynd run`) and a native binary
//! (`rynd build`). Follows the flows of the RealWorld API spec's Postman suite.
use rynd::{RyndEngine, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

#[path = "common/socket_lock.rs"]
mod socket_lock;

struct Server {
    child: Child,
    base: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("conduit");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn start(program: &[&str], db: &Path) -> Server {
    let mut child = Command::new(program[0])
        .args(&program[1..])
        .env("PORT", "0")
        .env("CONDUIT_DB", db)
        .env("JWT_SECRET", "test-secret")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start conduit");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let base = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .unwrap_or_else(|| panic!("no address in {line:?}"))
        .trim_end_matches("/api")
        .to_string();
    Server { child, base }
}

struct Response {
    status: u16,
    headers: String,
    body: Value,
}

fn json(text: &str) -> Value {
    if text.trim().is_empty() {
        return Value::Nil;
    }
    let mut engine = RyndEngine::new();
    engine.set_global("text", Value::string(text));
    engine.eval("parse_json(text)").expect(text)
}

fn request(
    base: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> Response {
    let addr = base.trim_start_matches("http://");
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    let body = body.unwrap_or("");
    let auth = token
        .map(|t| format!("Authorization: Token {t}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (head, rest) = raw.split_once("\r\n\r\n").expect("response head");
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    Response {
        status,
        headers: head.to_lowercase(),
        body: json(rest),
    }
}

/// Dotted lookup through maps and list indices: `"user.token"`, `"articles.0.slug"`.
fn at(value: &Value, path: &str) -> Value {
    let mut current = value.clone();
    for key in path.split('.') {
        current = match (&current, key.parse::<usize>()) {
            (Value::List(items), Ok(i)) => items.get(i).cloned().unwrap_or(Value::Nil),
            (Value::Map(map), _) => map.get(key).cloned().unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
    }
    current
}

fn s(value: Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, found {other}"),
    }
}

fn len(value: Value) -> usize {
    match value {
        Value::List(items) => items.len(),
        other => panic!("expected list, found {other}"),
    }
}

fn register(base: &str, name: &str) -> String {
    let body = format!(
        r#"{{"user":{{"username":"{name}","email":"{name}@example.com","password":"{name}-password"}}}}"#
    );
    let r = request(base, "POST", "/api/users", None, Some(&body));
    assert_eq!(r.status, 201, "register {name}: {}", r.body);
    s(at(&r.body, "user.token"))
}

fn scenario(base: &str) {
    // Users and authentication.
    let jake = register(base, "jake");
    let dup = request(
        base,
        "POST",
        "/api/users",
        None,
        Some(r#"{"user":{"username":"jake","email":"JAKE@example.com","password":"x"}}"#),
    );
    assert_eq!(dup.status, 422);
    assert_eq!(
        at(&dup.body, "errors.email.0"),
        Value::string("has already been taken")
    );
    assert_eq!(
        at(&dup.body, "errors.username.0"),
        Value::string("has already been taken")
    );
    let blank = request(
        base,
        "POST",
        "/api/users",
        None,
        Some(r#"{"user":{"email":""}}"#),
    );
    assert_eq!(blank.status, 422);
    assert_eq!(
        at(&blank.body, "errors.password.0"),
        Value::string("can't be blank")
    );
    assert_eq!(
        request(base, "POST", "/api/users", None, Some("not json")).status,
        422
    );

    let wrong = r#"{"user":{"email":"jake@example.com","password":"nope"}}"#;
    assert_eq!(
        request(base, "POST", "/api/users/login", None, Some(wrong)).status,
        401
    );
    let login = request(
        base,
        "POST",
        "/api/users/login",
        None,
        Some(r#"{"user":{"email":"jake@example.com","password":"jake-password"}}"#),
    );
    assert_eq!(login.status, 200);
    assert_eq!(at(&login.body, "user.username"), Value::string("jake"));
    assert_eq!(at(&login.body, "user.bio"), Value::Nil);

    assert_eq!(request(base, "GET", "/api/user", None, None).status, 401);
    assert_eq!(
        request(base, "GET", "/api/user", Some("bad.token.here"), None).status,
        401
    );
    let me = request(base, "GET", "/api/user", Some(&jake), None);
    assert_eq!(me.status, 200);
    assert_eq!(
        at(&me.body, "user.email"),
        Value::string("jake@example.com")
    );

    let updated = request(
        base,
        "PUT",
        "/api/user",
        Some(&jake),
        Some(
            r#"{"user":{"bio":"I like to skateboard","image":"https://i.stack.imgur.com/xHWG8.jpg"}}"#,
        ),
    );
    assert_eq!(updated.status, 200);
    assert_eq!(
        at(&updated.body, "user.bio"),
        Value::string("I like to skateboard")
    );

    // Profiles and following.
    let celeb = register(base, "celeb");
    let profile = request(base, "GET", "/api/profiles/celeb", Some(&jake), None);
    assert_eq!(at(&profile.body, "profile.following"), Value::Bool(false));
    let follow = request(
        base,
        "POST",
        "/api/profiles/celeb/follow",
        Some(&jake),
        None,
    );
    assert_eq!(at(&follow.body, "profile.following"), Value::Bool(true));
    assert_eq!(
        request(base, "GET", "/api/profiles/nobody", None, None).status,
        404
    );
    assert_eq!(
        request(base, "POST", "/api/profiles/celeb/follow", None, None).status,
        401
    );

    // Articles.
    let created = request(
        base,
        "POST",
        "/api/articles",
        Some(&celeb),
        Some(
            r#"{"article":{"title":"How to train your dragon","description":"Ever wonder how?","body":"You have to believe","tagList":["dragons","training"]}}"#,
        ),
    );
    assert_eq!(created.status, 201, "{}", created.body);
    let slug = s(at(&created.body, "article.slug"));
    assert_eq!(slug, "how-to-train-your-dragon");
    assert_eq!(
        at(&created.body, "article.tagList.1"),
        Value::string("training")
    );
    assert_eq!(
        at(&created.body, "article.author.username"),
        Value::string("celeb")
    );
    assert_eq!(at(&created.body, "article.favoritesCount"), Value::Int(0));
    let stamp = s(at(&created.body, "article.createdAt"));
    assert!(stamp.ends_with('Z') && stamp.contains('T'), "{stamp}");
    let second = request(
        base,
        "POST",
        "/api/articles",
        Some(&celeb),
        Some(
            r#"{"article":{"title":"How to train your dragon","description":"again","body":"b"}}"#,
        ),
    );
    assert_ne!(s(at(&second.body, "article.slug")), slug);
    assert_eq!(
        request(base, "POST", "/api/articles", None, Some("{}")).status,
        401
    );

    let feed = request(base, "GET", "/api/articles/feed", Some(&jake), None);
    assert_eq!(at(&feed.body, "articlesCount"), Value::Int(2));
    assert_eq!(
        at(&feed.body, "articles.0.author.following"),
        Value::Bool(true)
    );
    let by_tag = request(base, "GET", "/api/articles?tag=dragons", None, None);
    assert_eq!(at(&by_tag.body, "articlesCount"), Value::Int(1));
    let by_author = request(
        base,
        "GET",
        "/api/articles?author=celeb&limit=1",
        None,
        None,
    );
    assert_eq!(at(&by_author.body, "articlesCount"), Value::Int(2));
    assert_eq!(len(at(&by_author.body, "articles")), 1);
    let newest = s(at(&by_author.body, "articles.0.slug"));
    assert_ne!(newest, slug, "list is newest first");
    let offset = request(
        base,
        "GET",
        "/api/articles?author=celeb&limit=1&offset=1",
        None,
        None,
    );
    assert_eq!(s(at(&offset.body, "articles.0.slug")), slug);
    let none = request(base, "GET", "/api/articles?author=jake", None, None);
    assert_eq!(at(&none.body, "articlesCount"), Value::Int(0));

    // Favorites.
    let fav = request(
        base,
        "POST",
        &format!("/api/articles/{slug}/favorite"),
        Some(&jake),
        None,
    );
    assert_eq!(at(&fav.body, "article.favorited"), Value::Bool(true));
    assert_eq!(at(&fav.body, "article.favoritesCount"), Value::Int(1));
    let liked = request(base, "GET", "/api/articles?favorited=jake", None, None);
    assert_eq!(at(&liked.body, "articlesCount"), Value::Int(1));
    let unfav = request(
        base,
        "DELETE",
        &format!("/api/articles/{slug}/favorite"),
        Some(&jake),
        None,
    );
    assert_eq!(at(&unfav.body, "article.favoritesCount"), Value::Int(0));

    // Updating: only the author, and a new title moves the slug.
    let path = format!("/api/articles/{slug}");
    let change = r#"{"article":{"title":"Did you train your dragon?"}}"#;
    assert_eq!(
        request(base, "PUT", &path, Some(&jake), Some(change)).status,
        403
    );
    let changed = request(base, "PUT", &path, Some(&celeb), Some(change));
    assert_eq!(changed.status, 200);
    let slug = s(at(&changed.body, "article.slug"));
    assert_eq!(slug, "did-you-train-your-dragon");
    assert_eq!(
        at(&changed.body, "article.body"),
        Value::string("You have to believe")
    );
    let path = format!("/api/articles/{slug}");
    assert_eq!(request(base, "GET", &path, None, None).status, 200);

    // Comments.
    let comments = format!("{path}/comments");
    let comment = request(
        base,
        "POST",
        &comments,
        Some(&jake),
        Some(r#"{"comment":{"body":"Thank you so much!"}}"#),
    );
    assert_eq!(comment.status, 201);
    let id = match at(&comment.body, "comment.id") {
        Value::Int(id) => id,
        other => panic!("comment id {other}"),
    };
    let listed = request(base, "GET", &comments, None, None);
    assert_eq!(
        at(&listed.body, "comments.0.author.username"),
        Value::string("jake")
    );
    let one = format!("{comments}/{id}");
    assert_eq!(
        request(base, "DELETE", &one, Some(&celeb), None).status,
        403
    );
    assert_eq!(request(base, "DELETE", &one, Some(&jake), None).status, 204);
    assert_eq!(
        len(at(
            &request(base, "GET", &comments, None, None).body,
            "comments"
        )),
        0
    );

    // Tags, most used first.
    let tags = request(base, "GET", "/api/tags", None, None);
    assert_eq!(at(&tags.body, "tags"), json(r#"["dragons", "training"]"#));

    // Browsers preflight cross-origin requests.
    let preflight = request(base, "OPTIONS", "/api/articles", None, None);
    assert_eq!(preflight.status, 204);
    assert!(preflight.headers.contains("access-control-allow-origin: *"));
    assert_eq!(
        request(base, "PATCH", "/api/articles", None, None).status,
        405
    );
    assert_eq!(request(base, "GET", "/api/nothing", None, None).status, 404);

    // Deleting, which also unfollows nothing and keeps others' data.
    assert_eq!(
        request(base, "DELETE", &path, Some(&jake), None).status,
        403
    );
    assert_eq!(
        request(base, "DELETE", &path, Some(&celeb), None).status,
        204
    );
    assert_eq!(request(base, "GET", &path, None, None).status, 404);
    let unfollow = request(
        base,
        "DELETE",
        "/api/profiles/celeb/follow",
        Some(&jake),
        None,
    );
    assert_eq!(at(&unfollow.body, "profile.following"), Value::Bool(false));
}

/// A client that sends half a request and stalls must not block the others:
/// each connection is its own fiber, and blocked reads release the baton.
fn stalled_client_does_not_block(base: &str) {
    let addr = base.trim_start_matches("http://");
    let mut slow = TcpStream::connect(addr).unwrap();
    slow.write_all(b"GET /api/tags HTTP/1.1\r\nHost: x\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let started = std::time::Instant::now();
    let tags = request(base, "GET", "/api/tags", None, None);
    assert_eq!(tags.status, 200);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
    slow.write_all(b"\r\n").unwrap();
    let mut reply = String::new();
    slow.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
}

fn persists_across_restart(program: &[&str], db: &Path) {
    let server = start(program, db);
    let login = request(
        &server.base,
        "POST",
        "/api/users/login",
        None,
        Some(r#"{"user":{"email":"jake@example.com","password":"jake-password"}}"#),
    );
    assert_eq!(login.status, 200);
    assert_eq!(
        at(&login.body, "user.bio"),
        Value::string("I like to skateboard")
    );
    let list = request(&server.base, "GET", "/api/articles", None, None);
    assert_eq!(at(&list.body, "articlesCount"), Value::Int(1));
}

#[test]
fn conduit_on_the_vm() {
    let _guard = socket_lock::acquire().unwrap();
    let program = [
        env!("CARGO_BIN_EXE_rynd"),
        "run",
        "examples/conduit/main.rynd",
    ];
    let db = scratch("vm.db");
    {
        let server = start(&program, &db);
        scenario(&server.base);
        stalled_client_does_not_block(&server.base);
    }
    persists_across_restart(&program, &db);
}

#[test]
fn conduit_as_a_native_binary() {
    let _guard = socket_lock::acquire().unwrap();
    let binary = scratch("conduit-native");
    let built = Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args(["build", "examples/conduit/main.rynd", "-o"])
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let program = [binary.to_str().unwrap()];
    let db = scratch("native.db");
    {
        let server = start(&program, &db);
        scenario(&server.base);
        stalled_client_does_not_block(&server.base);
    }
    persists_across_restart(&program, &db);
}
