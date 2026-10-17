//! Hashing, encoding, randomness, password storage, and JWTs, std-only.
//!
//! SHA-256 (FIPS 180-4), HMAC (RFC 2104), PBKDF2 (RFC 8018), base64
//! (RFC 4648), UUIDv4 (RFC 9562), and HS256 JSON Web Tokens (RFC 7519).
//! Randomness comes from the operating system (`/dev/urandom`).
use super::{
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::convert::TryFrom;
use std::io::Read;

pub const BUILTINS: &[(&str, usize)] = &[
    ("sha256", 1),
    ("hmac_sha256", 2),
    ("base64_encode", 1),
    ("base64_decode", 1),
    ("base64_decode_bytes", 1),
    ("base64url_encode", 1),
    ("base64url_decode", 1),
    ("hex_encode", 1),
    ("to_bytes", 1),
    ("from_bytes", 1),
    ("random_bytes", 1),
    ("random_int", 2),
    ("random_token", 1),
    ("uuid", 0),
    ("secure_compare", 2),
    ("password_hash", 1),
    ("password_verify", 2),
    ("jwt_sign", 2),
    ("jwt_verify", 2),
];

/// PBKDF2-HMAC-SHA256 work factor for new password hashes. Stored hashes
/// record their own count, so raising this keeps old hashes verifiable.
pub const PASSWORD_ITERATIONS: u32 = 100_000;

// ---------------------------------------------------------------- SHA-256

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

#[derive(Clone)]
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: H0,
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                Self::compress(&mut self.state, &self.block);
                self.filled = 0;
            }
        }
    }

    fn finish(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (chunk, word) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finish()
}

/// HMAC with precomputed inner/outer states, so PBKDF2 rounds cost two
/// compressions each.
#[derive(Clone)]
struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    fn new(key: &[u8]) -> Self {
        let mut block = [0u8; 64];
        if key.len() > 64 {
            block[..32].copy_from_slice(&sha256(key));
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        let mut outer = Sha256::new();
        inner.update(&block.map(|b| b ^ 0x36));
        outer.update(&block.map(|b| b ^ 0x5c));
        Self { inner, outer }
    }

    fn mac(&self, data: &[u8]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        inner.update(data);
        let mut outer = self.outer.clone();
        outer.update(&inner.finish());
        outer.finish()
    }
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    Hmac::new(key).mac(data)
}

pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let prf = Hmac::new(password);
    let mut first = salt.to_vec();
    first.extend_from_slice(&1u32.to_be_bytes());
    let mut u = prf.mac(&first);
    let mut out = u;
    for _ in 1..iterations {
        u = prf.mac(&u);
        for (o, b) in out.iter_mut().zip(u) {
            *o ^= b;
        }
    }
    out
}

/// Compare without exiting early, so timing reveals only the lengths.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------- encoding

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_encode(data: &[u8], url: bool) -> String {
    let alphabet = if url { URL_SAFE } else { STANDARD };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=chunk.len() {
            out.push(alphabet[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
        if !url {
            for _ in chunk.len()..3 {
                out.push('=');
            }
        }
    }
    out
}

/// Accepts either alphabet, with or without padding; rejects anything else.
pub fn base64_decode(text: &str) -> RyndResult<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    let trimmed = text.trim_end_matches('=');
    for c in trimmed.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(error(format!("Invalid base64 character {:?}", c as char))),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if bits >= 6 || trimmed.len() % 4 == 1 {
        return Err(error("Invalid base64 length"));
    }
    Ok(out)
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Script data as bytes: strings are UTF-8, lists hold integers 0..=255.
pub fn bytes_of(value: &Value) -> RyndResult<Vec<u8>> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::Int(b) if (0..=255).contains(b) => Ok(*b as u8),
                _ => Err(error("Byte values must be integers in 0..255")),
            })
            .collect(),
        other => Err(error(format!(
            "Expected string or byte list, found {}",
            other.type_name()
        ))),
    }
}

fn byte_list(data: &[u8]) -> Value {
    Value::list(data.iter().map(|b| Value::Int(i64::from(*b))).collect())
}

fn utf8(data: Vec<u8>, builtin: &str) -> RyndResult<Value> {
    String::from_utf8(data)
        .map(Value::string)
        .map_err(|_| error(format!("{builtin}(): decoded bytes are not valid UTF-8")))
}

// ---------------------------------------------------------------- randomness

/// Fill `buf` from the operating system's CSPRNG.
pub fn fill_random(buf: &mut [u8]) -> RyndResult<()> {
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .map_err(|e| error(format!("System random source unavailable: {e}")))
}

pub fn random_vec(n: usize) -> RyndResult<Vec<u8>> {
    let mut buf = vec![0u8; n];
    fill_random(&mut buf)?;
    Ok(buf)
}

/// Uniform integer in `lo..hi` by rejection sampling (no modulo bias).
pub fn random_range(lo: i64, hi: i64) -> RyndResult<i64> {
    if lo >= hi {
        return Err(error("random_int() needs lo < hi"));
    }
    let span = (hi as i128 - lo as i128) as u128;
    let zone = u128::from(u64::MAX) + 1 - (u128::from(u64::MAX) + 1) % span;
    loop {
        let mut buf = [0u8; 8];
        fill_random(&mut buf)?;
        let n = u128::from(u64::from_le_bytes(buf));
        if n < zone {
            return Ok((lo as i128 + (n % span) as i128) as i64);
        }
    }
}

pub fn uuid_v4() -> RyndResult<String> {
    let mut b = [0u8; 16];
    fill_random(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

// ---------------------------------------------------------------- passwords

/// `$pbkdf2-sha256$<iterations>$<salt>$<hash>` with unpadded base64 fields.
pub fn password_hash(password: &str, iterations: u32) -> RyndResult<String> {
    let salt = random_vec(16)?;
    let hash = pbkdf2_sha256(password.as_bytes(), &salt, iterations);
    Ok(format!(
        "$pbkdf2-sha256${iterations}${}${}",
        base64_encode(&salt, true),
        base64_encode(&hash, true)
    ))
}

pub fn password_verify(password: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let ["", "pbkdf2-sha256", iterations, salt, hash] = parts.as_slice() else {
        return false;
    };
    let (Ok(iterations), Ok(salt), Ok(hash)) = (
        iterations.parse::<u32>(),
        base64_decode(salt),
        base64_decode(hash),
    ) else {
        return false;
    };
    if iterations == 0 || iterations > 10_000_000 {
        return false;
    }
    constant_time_eq(
        &pbkdf2_sha256(password.as_bytes(), &salt, iterations),
        &hash,
    )
}

// ---------------------------------------------------------------- JWT

pub fn jwt_sign(claims: &Value, secret: &[u8]) -> RyndResult<String> {
    if !matches!(claims, Value::Map(_)) {
        return Err(error("jwt_sign() expects a claims map"));
    }
    let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#, true);
    let payload = base64_encode(super::json::stringify(claims)?.as_bytes(), true);
    let signing_input = format!("{header}.{payload}");
    let signature = base64_encode(&hmac_sha256(secret, signing_input.as_bytes()), true);
    Ok(format!("{signing_input}.{signature}"))
}

/// The verified claims, or why the token is unacceptable. `exp` and `nbf`
/// are checked against the current time when present (seconds since epoch).
pub fn jwt_verify(token: &str, secret: &[u8]) -> Result<Value, String> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed token".into());
    };
    let header_json = base64_decode(header)
        .ok()
        .and_then(|h| String::from_utf8(h).ok())
        .ok_or("malformed header")?;
    match super::json::parse(&header_json) {
        Ok(Value::Map(h)) if h.get("alg") == Some(&Value::string("HS256")) => {}
        _ => return Err("unsupported algorithm".into()),
    }
    let expected = hmac_sha256(secret, format!("{header}.{payload}").as_bytes());
    let given = base64_decode(signature).map_err(|_| "malformed signature")?;
    if !constant_time_eq(&expected, &given) {
        return Err("invalid signature".into());
    }
    let claims_json = base64_decode(payload)
        .ok()
        .and_then(|p| String::from_utf8(p).ok())
        .ok_or("malformed payload")?;
    let claims = super::json::parse(&claims_json).map_err(|_| "malformed payload")?;
    let Value::Map(map) = &claims else {
        return Err("claims must be an object".into());
    };
    let now = super::scripting::unix_millis() / 1000;
    let seconds = |key: &str| match map.get(key) {
        Some(Value::Int(n)) => Some(*n),
        Some(Value::Float(f)) => Some(*f as i64),
        _ => None,
    };
    if seconds("exp").is_some_and(|exp| now >= exp) {
        return Err("token expired".into());
    }
    if seconds("nbf").is_some_and(|nbf| now < nbf) {
        return Err("token not yet valid".into());
    }
    Ok(claims)
}

// ---------------------------------------------------------------- builtins

fn text(value: &Value) -> RyndResult<&str> {
    <&str>::try_from(value)
}

pub fn call(_rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "sha256" => Ok(Value::string(hex(&sha256(&bytes_of(&args[0])?)))),
        "hmac_sha256" => Ok(Value::string(hex(&hmac_sha256(
            &bytes_of(&args[0])?,
            &bytes_of(&args[1])?,
        )))),
        "base64_encode" => Ok(Value::string(base64_encode(&bytes_of(&args[0])?, false))),
        "base64url_encode" => Ok(Value::string(base64_encode(&bytes_of(&args[0])?, true))),
        "base64_decode" | "base64url_decode" => utf8(base64_decode(text(&args[0])?)?, name),
        "base64_decode_bytes" => Ok(byte_list(&base64_decode(text(&args[0])?)?)),
        "hex_encode" => Ok(Value::string(hex(&bytes_of(&args[0])?))),
        "to_bytes" => Ok(byte_list(text(&args[0])?.as_bytes())),
        "from_bytes" => utf8(bytes_of(&args[0])?, name),
        "random_bytes" => {
            let n = usize::try_from(i64::try_from(&args[0])?)
                .map_err(|_| error("random_bytes() needs a non-negative count"))?;
            if n > 1 << 20 {
                return Err(error("random_bytes() is limited to 1 MiB per call"));
            }
            Ok(byte_list(&random_vec(n)?))
        }
        "random_int" => Ok(Value::Int(random_range(
            i64::try_from(&args[0])?,
            i64::try_from(&args[1])?,
        )?)),
        "random_token" => {
            let n = usize::try_from(i64::try_from(&args[0])?)
                .map_err(|_| error("random_token() needs a non-negative byte count"))?;
            if n > 1 << 16 {
                return Err(error("random_token() is limited to 64 KiB"));
            }
            Ok(Value::string(base64_encode(&random_vec(n)?, true)))
        }
        "uuid" => Ok(Value::string(uuid_v4()?)),
        "secure_compare" => Ok(Value::Bool(constant_time_eq(
            &bytes_of(&args[0])?,
            &bytes_of(&args[1])?,
        ))),
        "password_hash" => Ok(Value::string(password_hash(
            text(&args[0])?,
            PASSWORD_ITERATIONS,
        )?)),
        "password_verify" => Ok(Value::Bool(password_verify(
            text(&args[0])?,
            text(&args[1])?,
        ))),
        "jwt_sign" => Ok(Value::string(jwt_sign(&args[0], &bytes_of(&args[1])?)?)),
        "jwt_verify" => Ok(match jwt_verify(text(&args[0])?, &bytes_of(&args[1])?) {
            Ok(claims) => Value::variant("Ok", vec![claims]),
            Err(reason) => Value::variant("Err", vec![Value::string(reason)]),
        }),
        _ => Err(error(format!("Unknown crypto builtin '{name}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_fips_vectors() {
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex(&sha256(&vec![b'a'; 1_000_000])),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn hmac_matches_rfc4231() {
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn pbkdf2_matches_published_vectors() {
        assert_eq!(
            hex(&pbkdf2_sha256(b"password", b"salt", 1)),
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            hex(&pbkdf2_sha256(b"password", b"salt", 4096)),
            "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn base64_round_trips_rfc4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes(), false), encoded);
            assert_eq!(base64_decode(encoded).unwrap(), plain.as_bytes());
            let url = base64_encode(plain.as_bytes(), true);
            assert_eq!(url, encoded.trim_end_matches('='));
            assert_eq!(base64_decode(&url).unwrap(), plain.as_bytes());
        }
        assert!(base64_decode("Zm9v!").is_err());
        assert!(base64_decode("Z").is_err());
    }

    #[test]
    fn jwt_signature_matches_rfc7515_hs256_example_key() {
        // RFC 7515 appendix A.1 signing input and key.
        let key = base64_decode(
            "AyM1SysPpbyDfgZld3umj1qzKObwVMkoqQ-EstJQLr_T-1qS0gZH75aKtMN3Yj0iPS4hcgUuTwjAzZr1Z9CAow",
        )
        .unwrap();
        let input = "eyJ0eXAiOiJKV1QiLA0KICJhbGciOiJIUzI1NiJ9.eyJpc3MiOiJqb2UiLA0KICJleHAiOjEzMDA4MTkzODAsDQogImh0dHA6Ly9leGFtcGxlLmNvbS9pc19yb290Ijp0cnVlfQ";
        assert_eq!(
            base64_encode(&hmac_sha256(&key, input.as_bytes()), true),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        );
    }

    #[test]
    fn passwords_verify_only_with_the_right_secret() {
        let stored = password_hash("hunter2", 1000).unwrap();
        assert!(password_verify("hunter2", &stored));
        assert!(!password_verify("hunter3", &stored));
        assert!(!password_verify("hunter2", "plaintext"));
        assert_ne!(stored, password_hash("hunter2", 1000).unwrap());
    }

    #[test]
    fn random_values_are_well_formed() {
        let id = uuid_v4().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        for _ in 0..200 {
            let n = random_range(-3, 4).unwrap();
            assert!((-3..4).contains(&n));
        }
        assert!(random_range(5, 5).is_err());
    }
}
