//! Web Push, the browser's own channel for a notification while no Bagholder tab
//! is open (`docs/plans/notifications-through-the-browser.md`): the message
//! encrypted for the subscribing browser alone (RFC 8291, `aes128gcm`), signed
//! with this data folder's own key (RFC 8292, VAPID), and handed to that
//! browser's push service (RFC 8030). Built on `openssl`, which the server
//! already links: P-256 and its ECDH, HMAC-SHA-256 for HKDF, AES-128-GCM and
//! ES256.

use openssl::bn::{BigNum, BigNumContext};
use openssl::derive::Deriver;
use openssl::ec::{EcGroup, EcKey, EcPoint, PointConversionForm};
use openssl::ecdsa::EcdsaSig;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private, Public};
use openssl::sign::Signer;
use openssl::symm::{encrypt_aead, Cipher};

/// The record size every message is sent with: one record, never more.
pub const RECORD_SIZE: u32 = 4096;

/// The most a push service is asked to carry: 4096 octets of encrypted body, the
/// size every push service states it takes (RFC 8030 §7.2 requires at least it).
pub const MAX_BODY: usize = 4096;

/// What `sub` says of who sends: the software, never the person (brief 66, 1).
pub const SUBJECT: &str = "https://github.com/ProfessorBagholder/Bagholder";

fn group() -> EcGroup {
    EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).expect("P-256 is built into OpenSSL")
}

/// Base64url with no padding (RFC 4648 §5), as Web Push writes keys and tokens.
pub fn b64url(bytes: &[u8]) -> String {
    openssl::base64::encode_block(bytes).replace('+', "-").replace('/', "_").trim_end_matches('=').to_string()
}

/// Base64url read back, with or without padding and whitespace.
pub fn unb64url(text: &str) -> Result<Vec<u8>, String> {
    let mut s: String = text.chars().filter(|c| !c.is_whitespace()).map(|c| match c {
        '-' => '+',
        '_' => '/',
        c => c,
    }).collect();
    while s.len() % 4 != 0 {
        s.push('=');
    }
    openssl::base64::decode_block(&s).map_err(|e| format!("{text:?} is not base64url: {e}"))
}

/// A P-256 key pair: this data folder's VAPID key, or a message's own.
pub struct Keys {
    key: EcKey<Private>,
}

impl Keys {
    /// A new pair.
    pub fn generate() -> Keys {
        Keys { key: EcKey::generate(&group()).expect("a P-256 key is made") }
    }

    /// The pair whose private scalar is `private` (32 octets).
    pub fn from_private(private: &[u8]) -> Result<Keys, String> {
        let g = group();
        let d = BigNum::from_slice(private).map_err(|e| e.to_string())?;
        let mut ctx = BigNumContext::new().map_err(|e| e.to_string())?;
        let mut p = EcPoint::new(&g).map_err(|e| e.to_string())?;
        p.mul_generator2(&g, &d, &mut ctx).map_err(|e| e.to_string())?;
        let key = EcKey::from_private_components(&g, &d, &p).map_err(|e| e.to_string())?;
        key.check_key().map_err(|e| e.to_string())?;
        Ok(Keys { key })
    }

    /// The private scalar, 32 octets.
    pub fn private(&self) -> Vec<u8> {
        self.key.private_key().to_vec_padded(32).expect("a P-256 scalar fits 32 octets")
    }

    /// The public key, uncompressed (65 octets, `0x04 || x || y`).
    pub fn public(&self) -> Vec<u8> {
        let mut ctx = BigNumContext::new().expect("a context");
        self.key.public_key().to_bytes(&group(), PointConversionForm::UNCOMPRESSED, &mut ctx).expect("a point is written")
    }

    /// Whether `key` is a point on P-256: a browser's key that is not cannot be
    /// encrypted for, and is refused when it subscribes.
    pub fn is_point(key: &[u8]) -> bool {
        let g = group();
        BigNumContext::new().is_ok_and(|mut ctx| EcPoint::from_bytes(&g, key, &mut ctx).is_ok())
    }

    /// The secret shared with the holder of `peer` (ECDH).
    fn agree(&self, peer: &[u8]) -> Result<Vec<u8>, String> {
        let g = group();
        let mut ctx = BigNumContext::new().map_err(|e| e.to_string())?;
        let point = EcPoint::from_bytes(&g, peer, &mut ctx).map_err(|e| format!("the browser's key is not a P-256 point: {e}"))?;
        let theirs: PKey<Public> = PKey::from_ec_key(EcKey::from_public_key(&g, &point).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let ours = PKey::from_ec_key(self.key.clone()).map_err(|e| e.to_string())?;
        let mut d = Deriver::new(&ours).map_err(|e| e.to_string())?;
        d.set_peer(&theirs).map_err(|e| e.to_string())?;
        d.derive_to_vec().map_err(|e| e.to_string())
    }

    /// An ES256 signature of `data` (RFC 7518 §3.4: `r || s`, 32 octets each).
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let digest = openssl::sha::sha256(data);
        let sig = EcdsaSig::sign(&digest, &self.key).expect("a signature is made");
        let mut out = sig.r().to_vec_padded(32).expect("r fits");
        out.extend(sig.s().to_vec_padded(32).expect("s fits"));
        out
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let k = PKey::hmac(key).expect("an HMAC key");
    let mut s = Signer::new(MessageDigest::sha256(), &k).expect("an HMAC signer");
    s.update(data).expect("HMAC takes the data");
    s.sign_to_vec().expect("HMAC answers")
}

/// HKDF-SHA-256 (RFC 5869), for an output of at most 32 octets: one block.
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = hmac(salt, ikm);
    let mut t = info.to_vec();
    t.push(1);
    hmac(&prk, &t)[..len].to_vec()
}

/// `plaintext` encrypted for the browser whose subscription keys are `ua_public`
/// (65 octets) and `auth` (16 octets), with the message key `message` and `salt`
/// (16 octets): the request body, header and one record (RFC 8291 §3.4, §4).
pub fn encrypt_with(plaintext: &[u8], ua_public: &[u8], auth: &[u8], message: &Keys, salt: &[u8]) -> Result<Vec<u8>, String> {
    if salt.len() != 16 {
        return Err("the salt is 16 octets".into());
    }
    let as_public = message.public();
    let ecdh = message.agree(ua_public)?;
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(&as_public);
    let ikm = hkdf(auth, &ecdh, &key_info, 32);
    let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);
    // the last (and only) record: the plaintext and its delimiter
    let mut padded = plaintext.to_vec();
    padded.push(2);
    let mut tag = [0u8; 16];
    let mut sealed = encrypt_aead(Cipher::aes_128_gcm(), &cek, Some(&nonce), &[], &padded, &mut tag).map_err(|e| e.to_string())?;
    sealed.extend_from_slice(&tag);
    let mut body = salt.to_vec();
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(&as_public);
    body.extend(sealed);
    Ok(body)
}

/// `plaintext` encrypted with a fresh message key and salt.
pub fn encrypt(plaintext: &[u8], ua_public: &[u8], auth: &[u8]) -> Result<Vec<u8>, String> {
    let mut salt = [0u8; 16];
    openssl::rand::rand_bytes(&mut salt).map_err(|e| e.to_string())?;
    encrypt_with(plaintext, ua_public, auth, &Keys::generate(), &salt)
}

/// The longest plaintext that fits one push: the body less its header (86
/// octets), the delimiter and the tag.
pub const MAX_PLAINTEXT: usize = MAX_BODY - 86 - 1 - 16;

/// A VAPID token (RFC 8292 §2): an ES256 JWT for the push service at `audience`
/// (its origin), good until `expires` (Unix seconds), naming the software.
pub fn vapid_token(key: &Keys, audience: &str, expires: u64) -> String {
    let header = b64url(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = b64url(serde_json::json!({ "aud": audience, "exp": expires, "sub": SUBJECT }).to_string().as_bytes());
    let signed = format!("{header}.{claims}");
    format!("{signed}.{}", b64url(&key.sign(signed.as_bytes())))
}

/// The `Authorization` header a push carries (RFC 8292 §3).
pub fn authorization(token: &str, key: &Keys) -> String {
    format!("vapid t={token}, k={}", b64url(&key.public()))
}

/// The origin of a push endpoint: what its token's `aud` names.
pub fn origin_of(endpoint: &str) -> Result<String, String> {
    let rest = endpoint.strip_prefix("https://").map(|r| ("https", r)).or_else(|| endpoint.strip_prefix("http://").map(|r| ("http", r)));
    let Some((scheme, rest)) = rest else { return Err(format!("{endpoint:?} is not an http(s) URL")) };
    let host = rest.split('/').next().unwrap_or("");
    if host.is_empty() {
        return Err(format!("{endpoint:?} names no host"));
    }
    Ok(format!("{scheme}://{host}"))
}

/// A push body opened as the browser holding `ua` and `auth` opens it: what a
/// test's stand-in push service checks it was sent (RFC 8291 §3.4, read back).
#[cfg(test)]
pub(crate) fn decrypt(body: &[u8], ua: &Keys, auth: &[u8]) -> Result<Vec<u8>, String> {
    if body.len() < 86 + 17 {
        return Err("too short for a header and a record".into());
    }
    let (salt, rs, idlen) = (&body[..16], u32::from_be_bytes(body[16..20].try_into().unwrap()), body[20] as usize);
    if rs != RECORD_SIZE || idlen != 65 {
        return Err(format!("record size {rs}, key id of {idlen} octets"));
    }
    let as_public = &body[21..21 + idlen];
    let sealed = &body[21 + idlen..];
    let ecdh = ua.agree(as_public)?;
    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&ua.public());
    key_info.extend_from_slice(as_public);
    let ikm = hkdf(auth, &ecdh, &key_info, 32);
    let cek = hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = hkdf(salt, &ikm, b"Content-Encoding: nonce\0", 12);
    let (data, tag) = sealed.split_at(sealed.len() - 16);
    let mut plain = openssl::symm::decrypt_aead(Cipher::aes_128_gcm(), &cek, Some(&nonce), &[], data, tag).map_err(|e| e.to_string())?;
    // the padding delimiter of the last record
    while plain.last() == Some(&0) {
        plain.pop();
    }
    if plain.pop() != Some(2) {
        return Err("no last-record delimiter".into());
    }
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Vec<u8> {
        unb64url(s).unwrap()
    }

    /// RFC 8291's worked example (§5 and Appendix A), reproduced byte for byte: from
    /// its keys, salt and plaintext, the same shared secret, keys, nonce and body.
    #[test]
    fn rfc_8291_appendix_a_byte_for_byte() {
        let plaintext = b("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");
        let as_key = Keys::from_private(&b("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        assert_eq!(as_key.public(), b("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"));
        let ua_public = b("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let salt = b("DGv6ra1nlYgDCS1FRnbzlw");
        let auth = b("BTBZMqHH6r4Tts7J_aSIgg");
        // the intermediate values
        let ecdh = as_key.agree(&ua_public).unwrap();
        assert_eq!(ecdh, b("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"));
        assert_eq!(hmac(&auth, &ecdh), b("Snr3JMxaHVDXHWJn5wdC52WjpCtd2EIEGBykDcZW32k"), "PRK_key");
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend(&ua_public);
        key_info.extend(as_key.public());
        assert_eq!(key_info, b("V2ViUHVzaDogaW5mbwAEJXGyvs3942BVGq8e0PTNNmwRzr5VX4m8t7GGpTM5FzFo7OLr4BhZe9MEebhuPI-OztV3ylkYfpJGmQ22ggCLDgT-M_SrDepxkU21WCP3O1SUj0EwbZIHMtu5pZpTKGSCIA5Zent7wmC6HCJ5mFgJkuk5cwAvMBKiiujwa7t45ewP"));
        let ikm = hkdf(&auth, &ecdh, &key_info, 32);
        assert_eq!(ikm, b("S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg"), "IKM");
        assert_eq!(hmac(&salt, &ikm), b("09_eUZGrsvxChDCGRCdkLiDXrReGOEVeSCdCcPBSJSc"), "PRK");
        assert_eq!(hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16), b("oIhVW04MRdy2XN9CiKLxTg"), "CEK");
        assert_eq!(hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", 12), b("4h_95klXJ5E_qnoN"), "NONCE");
        // the body: the 86-octet header, then the ciphertext
        let body = encrypt_with(&plaintext, &ua_public, &auth, &as_key, &salt).unwrap();
        let header = b("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
        assert_eq!(header.len(), 86);
        assert_eq!(&body[..86], &header[..]);
        assert_eq!(&body[86..], &b("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ")[..]);
    }

    /// A VAPID token has RFC 8292's header and claims, and verifies against the
    /// public key it is sent with.
    #[test]
    fn a_vapid_token_has_rfc_8292s_form_and_verifies() {
        let key = Keys::generate();
        let token = vapid_token(&key, "https://push.example.net", 1_800_000_000);
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&unb64url(parts[0]).unwrap()).unwrap();
        assert_eq!(header, serde_json::json!({"typ": "JWT", "alg": "ES256"}));
        let claims: serde_json::Value = serde_json::from_slice(&unb64url(parts[1]).unwrap()).unwrap();
        assert_eq!(claims, serde_json::json!({"aud": "https://push.example.net", "exp": 1_800_000_000u64, "sub": SUBJECT}));
        let sig = unb64url(parts[2]).unwrap();
        assert_eq!(sig.len(), 64, "r || s");
        let ecdsa = EcdsaSig::from_private_components(BigNum::from_slice(&sig[..32]).unwrap(), BigNum::from_slice(&sig[32..]).unwrap()).unwrap();
        let mut ctx = BigNumContext::new().unwrap();
        let public = EcKey::from_public_key(&group(), &EcPoint::from_bytes(&group(), &key.public(), &mut ctx).unwrap()).unwrap();
        assert!(ecdsa.verify(&openssl::sha::sha256(format!("{}.{}", parts[0], parts[1]).as_bytes()), &public).unwrap());
        assert_eq!(authorization(&token, &key), format!("vapid t={token}, k={}", b64url(&key.public())));
        assert_eq!(origin_of("https://fcm.googleapis.com/fcm/send/abc").unwrap(), "https://fcm.googleapis.com");
        assert_eq!(origin_of("http://127.0.0.1:9123/push/x").unwrap(), "http://127.0.0.1:9123");
    }
}
