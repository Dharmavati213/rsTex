//! Reading encrypted PDF files: xpdf's Decrypt.cc and the Standard handler of SecurityHandler.cc, with the empty user
//! password, which is all `read_pdf_info` ever tries.  pdfTeX copies what it decrypts, so this matters to the output.
//!
//! RC4, AES and SHA-2 here are the published algorithms, written for this and nothing else: speed does not matter
//! for the rare file that needs them.  MD5 is rstex_pdfio's.

use crate::pdf::{Doc, Object};
use rstex_pdfio::md5::md5;

const PASSWORD_PAD: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08, 0x2e, 0x2e, 0x00, 0xb6, 0xd0,
    0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

/// `CryptAlgorithm`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Algo {
    Rc4,
    Aes,
    Aes256,
}

/// What `XRef::setEncryption` keeps.
#[derive(Clone, Debug)]
pub(crate) struct Crypt {
    pub file_key: [u8; 32],
    pub key_length: usize,
    pub algo: Algo,
}

/// The outcome of `PDFDoc::checkEncryption`.
pub(crate) enum Check {
    /// `isUnencrypted`.
    None,
    /// Authorised with the empty user password, printing allowed or not.
    Key(Crypt, bool),
    /// No handler, or the password does not work.
    Refused,
}

/// `SecurityHandler::make`, `StandardSecurityHandler::StandardSecurityHandler`, `checkEncryption`.
pub(crate) fn check_encryption(doc: &mut Doc, enc: &crate::pdf::Dict, id: &Object) -> Check {
    if !doc.lookup(enc, b"Filter").is_name(b"Standard") {
        return Check::Refused;
    }
    let version = doc.lookup(enc, b"V");
    let revision = doc.lookup(enc, b"R");
    let length = doc.lookup(enc, b"Length");
    let owner_key = doc.lookup(enc, b"O");
    let user_key = doc.lookup(enc, b"U");
    let owner_enc = doc.lookup(enc, b"OE");
    let user_enc = doc.lookup(enc, b"UE");
    let perm = doc.lookup(enc, b"P");
    // Parameters of the wrong type leave encVersion and encRevision at -1, which isUnencrypted() takes for no
    // encryption, so such a file opens and is read as it stands.
    let (Object::Int(mut v), Object::Int(mut r), Object::Int(perm), Object::Str(okey), Object::Str(ukey)) =
        (version, revision, perm, owner_key, user_key)
    else {
        return Check::None;
    };
    if v == -1 && r == -1 {
        return Check::None;
    }
    let mut algo = Algo::Rc4;
    let mut file_key_length = match length {
        Object::Int(n) if r != 2 => n / 8,
        _ => 5,
    };
    let mut encrypt_metadata = true;
    if (v == 4 || v == 5) && (r == 4 || r == 5 || r == 6) {
        let cf = doc.lookup(enc, b"CF");
        let stmf = doc.lookup(enc, b"StmF");
        let strf = doc.lookup(enc, b"StrF");
        if let (Object::Dict(cf), Object::Name(stmf), Object::Name(strf)) = (&cf, &stmf, &strf) {
            if stmf == strf {
                if stmf.as_slice() == b"Identity" {
                    // "no encryption on streams or strings", says xpdf, and goes to `done` with `ok` false: refused.
                    return Check::Refused;
                }
                if let Object::Dict(filter) = doc.lookup(cf, stmf) {
                    let cfm = doc.lookup(&filter, b"CFM");
                    let cf_len = doc.lookup(&filter, b"Length");
                    if cfm.is_name(b"V2") {
                        if let Object::Int(n) = cf_len {
                            file_key_length = n;
                        }
                        v = 2;
                        r = 3;
                    } else if cfm.is_name(b"AESV2") {
                        if let Object::Int(n) = cf_len {
                            file_key_length = n;
                        }
                        v = 2;
                        r = 3;
                        algo = Algo::Aes;
                    } else if cfm.is_name(b"AESV3") {
                        v = 5;
                        if r != 5 && r != 6 {
                            r = 6;
                        }
                        algo = Algo::Aes256;
                        file_key_length = 32;
                    }
                }
            }
        }
        if let Object::Bool(b) = doc.lookup(enc, b"EncryptMetadata") {
            encrypt_metadata = b;
        }
    }
    let mut okey = okey.as_ref().clone();
    let mut ukey = ukey.as_ref().clone();
    if r <= 4 {
        okey.resize(okey.len().max(32), 0);
        ukey.resize(ukey.len().max(32), 0);
    } else if r <= 6 {
        let (Object::Str(oe), Object::Str(ue)) = (&owner_enc, &user_enc) else {
            return Check::Refused;
        };
        if okey.len() < 48 || ukey.len() < 48 || oe.len() != 32 || ue.len() != 32 {
            return Check::Refused;
        }
    }
    let mut file_key = [0u8; 32];
    let ok;
    if (1..=2).contains(&v) && (2..=3).contains(&r) {
        let file_id: Vec<u8> = match id {
            Object::Array(a) => match doc.array_get(a, 0) {
                Object::Str(s) => s.as_ref().clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        if file_key_length > 16 || file_key_length <= 0 {
            file_key_length = 16;
        }
        ok = make_file_key2(r, file_key_length as usize, &okey, &ukey, perm, &file_id, &mut file_key, encrypt_metadata);
    } else if v == 5 && (r == 5 || r == 6) {
        if file_key_length > 32 || file_key_length <= 0 {
            file_key_length = 32;
        }
        let Object::Str(ue) = &user_enc else {
            return Check::Refused;
        };
        ok = make_file_key_v5(r, &ukey, ue, &mut file_key);
    } else {
        return Check::Refused;
    }
    if !ok {
        return Check::Refused;
    }
    let key_length = (file_key_length as usize).min(32);
    // XRef::okToPrint: no owner password was tried.
    let may_print = perm & (1 << 2) != 0;
    Check::Key(Crypt { file_key, key_length, algo }, may_print)
}

/// `Decrypt::makeFileKey2` with no password given.
#[allow(clippy::too_many_arguments)]
fn make_file_key2(
    r: i32,
    key_length: usize,
    owner_key: &[u8],
    user_key: &[u8],
    permissions: i32,
    file_id: &[u8],
    file_key: &mut [u8; 32],
    encrypt_metadata: bool,
) -> bool {
    let mut buf = Vec::with_capacity(72 + file_id.len());
    buf.extend_from_slice(&PASSWORD_PAD);
    buf.extend_from_slice(&owner_key[..32]);
    buf.extend_from_slice(&(permissions as u32).to_le_bytes());
    buf.extend_from_slice(file_id);
    if !encrypt_metadata {
        buf.extend_from_slice(&[0xff; 4]);
    }
    let mut key = md5(&buf);
    if r == 3 {
        for _ in 0..50 {
            key = md5(&key[..key_length]);
        }
    }
    file_key[..16].copy_from_slice(&key);
    if r == 2 {
        let mut test = user_key[..32].to_vec();
        rc4(&key[..key_length], &mut test);
        test == PASSWORD_PAD
    } else if r == 3 {
        let mut test = user_key[..32].to_vec();
        for i in (0..20u8).rev() {
            let tmp: Vec<u8> = key[..key_length].iter().map(|b| b ^ i).collect();
            rc4(&tmp, &mut test);
        }
        let mut b = PASSWORD_PAD.to_vec();
        b.extend_from_slice(file_id);
        test[..16] == md5(&b)
    } else {
        false
    }
}

/// `Decrypt::makeFileKey`, revisions 5 and 6, the empty user password.
fn make_file_key_v5(r: i32, user_key: &[u8], user_enc: &[u8], file_key: &mut [u8; 32]) -> bool {
    let mut test = sha256(&user_key[32..40]).to_vec();
    if r == 6 {
        test = r6_hash(test);
    }
    if test[..32] != user_key[..32] {
        return false;
    }
    let mut test = sha256(&user_key[40..48]).to_vec();
    if r == 6 {
        test = r6_hash(test);
    }
    let dec = aes_cbc_decrypt_raw(&test[..32], &[0u8; 16], &user_enc[..32]);
    file_key[..32].copy_from_slice(&dec);
    true
}

/// `Decrypt::r6Hash`, with an empty password and no user key.
fn r6_hash(mut key: Vec<u8>) -> Vec<u8> {
    let mut i = 0i32;
    loop {
        let unit = key.clone();
        let mut key1 = Vec::with_capacity(unit.len() * 64);
        for _ in 0..64 {
            key1.extend_from_slice(&unit);
        }
        let n = key1.len();
        let w = expand_key(&key[..16]);
        let mut cbc = [0u8; 16];
        cbc.copy_from_slice(&key[16..32]);
        let mut j = 0;
        while j + 16 <= n {
            for k in 0..16 {
                key1[j + k] ^= cbc[k];
            }
            let mut b = [0u8; 16];
            b.copy_from_slice(&key1[j..j + 16]);
            aes_encrypt_block(&w, &mut b);
            key1[j..j + 16].copy_from_slice(&b);
            cbc = b;
            j += 16;
        }
        let mut k = 0u32;
        for &b in &key1[..16] {
            k += (b % 3) as u32;
        }
        key = match k % 3 {
            0 => sha256(&key1).to_vec(),
            1 => sha512_family(&key1, true),
            _ => sha512_family(&key1, false),
        };
        i += 1;
        if i >= 64 && (key1[n - 1] as i32) <= i - 32 {
            break;
        }
    }
    key.truncate(32);
    key
}

/// `DecryptStream`: the object key, then all of `data`.
pub(crate) fn decrypt(c: &Crypt, num: i32, gen_no: i32, data: &[u8]) -> Vec<u8> {
    let mut obj_key = c.file_key[..c.key_length].to_vec();
    match c.algo {
        Algo::Rc4 | Algo::Aes => {
            obj_key.extend_from_slice(&[num as u8, (num >> 8) as u8, (num >> 16) as u8, gen_no as u8, (gen_no >> 8) as u8]);
            if c.algo == Algo::Aes {
                obj_key.extend_from_slice(b"sAlT");
            }
            let h = md5(&obj_key);
            obj_key = h[..(c.key_length + 5).min(16)].to_vec();
        }
        Algo::Aes256 => {}
    }
    match c.algo {
        Algo::Rc4 => {
            let mut out = data.to_vec();
            rc4(&obj_key, &mut out);
            out
        }
        _ => {
            // reset: the first block is the IV.  getChar: only whole blocks, with the padding off the last.
            if data.len() < 16 {
                return Vec::new();
            }
            let mut iv = [0u8; 16];
            iv.copy_from_slice(&data[..16]);
            let body = &data[16..];
            let whole = body.len() / 16 * 16;
            let plain = aes_cbc_decrypt_raw(&obj_key, &iv, &body[..whole]);
            let mut out = plain;
            // str->lookChar() == EOF after the block: a partial block at the end means the whole ones were not last.
            if whole > 0 && whole == body.len() {
                let last = whole - 16;
                let mut n = out[whole - 1] as usize;
                if !(1..=16).contains(&n) {
                    n = 16;
                }
                // The C moves the block up by n and starts at n: the first 16-n bytes of the block stay.
                out.truncate(last + 16 - n);
            }
            out
        }
    }
}

fn rc4(key: &[u8], data: &mut [u8]) {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 255;
        s.swap(i, j);
    }
    let (mut x, mut y) = (0usize, 0usize);
    for b in data.iter_mut() {
        x = (x + 1) & 255;
        y = (y + s[x] as usize) & 255;
        s.swap(x, y);
        *b ^= s[(s[x] as usize + s[y] as usize) & 255];
    }
}

const fn aes_tables() -> ([u8; 256], [u8; 256]) {
    let mut sbox = [0u8; 256];
    let mut inv = [0u8; 256];
    let (mut p, mut q) = (1u8, 1u8);
    loop {
        p = p ^ (p << 1) ^ (if p & 0x80 != 0 { 0x1b } else { 0 });
        q ^= q << 1;
        q ^= q << 2;
        q ^= q << 4;
        if q & 0x80 != 0 {
            q ^= 0x09;
        }
        let x = q ^ q.rotate_left(1) ^ q.rotate_left(2) ^ q.rotate_left(3) ^ q.rotate_left(4);
        sbox[p as usize] = x ^ 0x63;
        inv[(x ^ 0x63) as usize] = p;
        if p == 1 {
            break;
        }
    }
    sbox[0] = 0x63;
    inv[0x63] = 0;
    (sbox, inv)
}

const AES_TABLES: ([u8; 256], [u8; 256]) = aes_tables();

fn xtime(b: u8) -> u8 {
    (b << 1) ^ (if b & 0x80 != 0 { 0x1b } else { 0 })
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut r = 0;
    while b != 0 {
        if b & 1 != 0 {
            r ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    r
}

/// The round keys, as bytes.  `key` is 16 or 32 bytes.
fn expand_key(key: &[u8]) -> Vec<[u8; 4]> {
    let nk = key.len() / 4;
    let rounds = nk + 6;
    let mut w: Vec<[u8; 4]> = key.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    let mut rcon = 1u8;
    for i in nk..4 * (rounds + 1) {
        let mut t = w[i - 1];
        if i % nk == 0 {
            t = [
                AES_TABLES.0[t[1] as usize] ^ rcon,
                AES_TABLES.0[t[2] as usize],
                AES_TABLES.0[t[3] as usize],
                AES_TABLES.0[t[0] as usize],
            ];
            rcon = xtime(rcon);
        } else if nk > 6 && i % nk == 4 {
            t = [
                AES_TABLES.0[t[0] as usize],
                AES_TABLES.0[t[1] as usize],
                AES_TABLES.0[t[2] as usize],
                AES_TABLES.0[t[3] as usize],
            ];
        }
        let p = w[i - nk];
        w.push([p[0] ^ t[0], p[1] ^ t[1], p[2] ^ t[2], p[3] ^ t[3]]);
    }
    w
}

fn add_round_key(s: &mut [u8; 16], w: &[[u8; 4]], round: usize) {
    for c in 0..4 {
        for r in 0..4 {
            s[c * 4 + r] ^= w[round * 4 + c][r];
        }
    }
}

fn aes_encrypt_block(w: &[[u8; 4]], s: &mut [u8; 16]) {
    let rounds = w.len() / 4 - 1;
    add_round_key(s, w, 0);
    for round in 1..=rounds {
        for b in s.iter_mut() {
            *b = AES_TABLES.0[*b as usize];
        }
        let t = *s;
        for c in 0..4 {
            for r in 0..4 {
                s[c * 4 + r] = t[((c + r) % 4) * 4 + r];
            }
        }
        if round != rounds {
            for c in 0..4 {
                let a = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
                s[c * 4] = xtime(a[0]) ^ xtime(a[1]) ^ a[1] ^ a[2] ^ a[3];
                s[c * 4 + 1] = a[0] ^ xtime(a[1]) ^ xtime(a[2]) ^ a[2] ^ a[3];
                s[c * 4 + 2] = a[0] ^ a[1] ^ xtime(a[2]) ^ xtime(a[3]) ^ a[3];
                s[c * 4 + 3] = xtime(a[0]) ^ a[0] ^ a[1] ^ a[2] ^ xtime(a[3]);
            }
        }
        add_round_key(s, w, round);
    }
}

fn aes_decrypt_block(w: &[[u8; 4]], s: &mut [u8; 16]) {
    let rounds = w.len() / 4 - 1;
    add_round_key(s, w, rounds);
    for round in (0..rounds).rev() {
        let t = *s;
        for c in 0..4 {
            for r in 0..4 {
                s[((c + r) % 4) * 4 + r] = t[c * 4 + r];
            }
        }
        for b in s.iter_mut() {
            *b = AES_TABLES.1[*b as usize];
        }
        add_round_key(s, w, round);
        if round != 0 {
            for c in 0..4 {
                let a = [s[c * 4], s[c * 4 + 1], s[c * 4 + 2], s[c * 4 + 3]];
                s[c * 4] = gmul(a[0], 14) ^ gmul(a[1], 11) ^ gmul(a[2], 13) ^ gmul(a[3], 9);
                s[c * 4 + 1] = gmul(a[0], 9) ^ gmul(a[1], 14) ^ gmul(a[2], 11) ^ gmul(a[3], 13);
                s[c * 4 + 2] = gmul(a[0], 13) ^ gmul(a[1], 9) ^ gmul(a[2], 14) ^ gmul(a[3], 11);
                s[c * 4 + 3] = gmul(a[0], 11) ^ gmul(a[1], 13) ^ gmul(a[2], 9) ^ gmul(a[3], 14);
            }
        }
    }
}

/// CBC over whole blocks, keeping any padding.
fn aes_cbc_decrypt_raw(key: &[u8], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let w = expand_key(key);
    let mut out = Vec::with_capacity(data.len());
    let mut prev = *iv;
    for block in data.chunks_exact(16) {
        let mut b = [0u8; 16];
        b.copy_from_slice(block);
        let keep = b;
        aes_decrypt_block(&w, &mut b);
        for k in 0..16 {
            b[k] ^= prev[k];
        }
        out.extend_from_slice(&b);
        prev = keep;
    }
    out
}

fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    let mut w = [0u32; 64];
    for block in msg.chunks(64) {
        for (i, c) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

/// SHA-512, or SHA-384 with `short`.
fn sha512_family(data: &[u8], short: bool) -> Vec<u8> {
    const K: [u64; 80] = [
        0x428a2f98d728ae22, 0x7137449123ef65cd, 0xb5c0fbcfec4d3b2f, 0xe9b5dba58189dbbc, 0x3956c25bf348b538, 0x59f111f1b605d019,
        0x923f82a4af194f9b, 0xab1c5ed5da6d8118, 0xd807aa98a3030242, 0x12835b0145706fbe, 0x243185be4ee4b28c, 0x550c7dc3d5ffb4e2,
        0x72be5d74f27b896f, 0x80deb1fe3b1696b1, 0x9bdc06a725c71235, 0xc19bf174cf692694, 0xe49b69c19ef14ad2, 0xefbe4786384f25e3,
        0x0fc19dc68b8cd5b5, 0x240ca1cc77ac9c65, 0x2de92c6f592b0275, 0x4a7484aa6ea6e483, 0x5cb0a9dcbd41fbd4, 0x76f988da831153b5,
        0x983e5152ee66dfab, 0xa831c66d2db43210, 0xb00327c898fb213f, 0xbf597fc7beef0ee4, 0xc6e00bf33da88fc2, 0xd5a79147930aa725,
        0x06ca6351e003826f, 0x142929670a0e6e70, 0x27b70a8546d22ffc, 0x2e1b21385c26c926, 0x4d2c6dfc5ac42aed, 0x53380d139d95b3df,
        0x650a73548baf63de, 0x766a0abb3c77b2a8, 0x81c2c92e47edaee6, 0x92722c851482353b, 0xa2bfe8a14cf10364, 0xa81a664bbc423001,
        0xc24b8b70d0f89791, 0xc76c51a30654be30, 0xd192e819d6ef5218, 0xd69906245565a910, 0xf40e35855771202a, 0x106aa07032bbd1b8,
        0x19a4c116b8d2d0c8, 0x1e376c085141ab53, 0x2748774cdf8eeb99, 0x34b0bcb5e19b48a8, 0x391c0cb3c5c95a63, 0x4ed8aa4ae3418acb,
        0x5b9cca4f7763e373, 0x682e6ff3d6b2b8a3, 0x748f82ee5defb2fc, 0x78a5636f43172f60, 0x84c87814a1f0ab72, 0x8cc702081a6439ec,
        0x90befffa23631e28, 0xa4506cebde82bde9, 0xbef9a3f7b2c67915, 0xc67178f2e372532b, 0xca273eceea26619c, 0xd186b8c721c0c207,
        0xeada7dd6cde0eb1e, 0xf57d4f7fee6ed178, 0x06f067aa72176fba, 0x0a637dc5a2c898a6, 0x113f9804bef90dae, 0x1b710b35131c471b,
        0x28db77f523047d84, 0x32caab7b40c72493, 0x3c9ebe0a15c9bebc, 0x431d67c49c100d4c, 0x4cc5d4becb3e42b6, 0x597f299cfc657e2a,
        0x5fcb6fab3ad6faec, 0x6c44198c4a475817,
    ];
    let mut h: [u64; 8] = if short {
        [
            0xcbbb9d5dc1059ed8, 0x629a292a367cd507, 0x9159015a3070dd17, 0x152fecd8f70e5939, 0x67332667ffc00b31, 0x8eb44a8768581511,
            0xdb0c2e0d64f98fa7, 0x47b5481dbefa4fa4,
        ]
    } else {
        [
            0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1, 0x510e527fade682d1, 0x9b05688c2b3e6c1f,
            0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
        ]
    };
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 128 != 112 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u128) * 8).to_be_bytes());
    let mut w = [0u64; 80];
    for block in msg.chunks(128) {
        for (i, c) in block.chunks(8).enumerate() {
            w[i] = u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..80 {
            let s1 = v[4].rotate_right(14) ^ v[4].rotate_right(18) ^ v[4].rotate_right(41);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(28) ^ v[0].rotate_right(34) ^ v[0].rotate_right(39);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = Vec::with_capacity(64);
    for x in h.iter().take(if short { 6 } else { 8 }) {
        out.extend_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn digests() {
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hex(&sha512_family(b"abc", false)),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            hex(&sha512_family(b"abc", true)),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
    }

    #[test]
    fn rc4_vector() {
        let mut d = b"Plaintext".to_vec();
        rc4(b"Key", &mut d);
        assert_eq!(hex(&d), "bbf316e8d940af0ad3");
    }

    #[test]
    fn aes_vectors() {
        // FIPS 197, appendix C.
        let key128: Vec<u8> = (0..16).collect();
        let w = expand_key(&key128);
        let plain: [u8; 16] = [0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let mut s = plain;
        aes_encrypt_block(&w, &mut s);
        assert_eq!(hex(&s), "69c4e0d86a7b0430d8cdb78070b4c55a");
        aes_decrypt_block(&w, &mut s);
        assert_eq!(s, plain);
        let key256: Vec<u8> = (0..32).collect();
        let w = expand_key(&key256);
        let mut s = plain;
        aes_encrypt_block(&w, &mut s);
        assert_eq!(hex(&s), "8ea2b7ca516745bfeafc49904b496089");
        aes_decrypt_block(&w, &mut s);
        assert_eq!(s, plain);
    }
}
