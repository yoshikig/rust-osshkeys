use super::{Key, PrivKey, PubKey};
use crate::error::Error;
use crate::sshbuf::{SshReadExt, SshWriteExt};
use openssl::bn::BigNumContext;
use openssl::ec::{EcGroup, EcGroupRef, EcKey, EcKeyRef, EcPointRef, PointConversionForm};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{HasParams, HasPublic, PKey, Private, Public};
use openssl::sign::{Signer, Verifier};
use std::convert::TryInto;
use std::fmt;
use std::io::Cursor;
use std::str::FromStr;

const NIST_P256_NAME: &'static str = "ecdsa-sha2-nistp256";
const NIST_P384_NAME: &'static str = "ecdsa-sha2-nistp384";
const NIST_P521_NAME: &'static str = "ecdsa-sha2-nistp521";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcCurve {
    Nistp256,
    Nistp384,
    Nistp521,
}

impl EcCurve {
    pub fn size(&self) -> usize {
        match self {
            EcCurve::Nistp256 => 256,
            EcCurve::Nistp384 => 384,
            EcCurve::Nistp521 => 521,
        }
    }

    pub fn nid(&self) -> Nid {
        match self {
            EcCurve::Nistp256 => Nid::X9_62_PRIME256V1,
            EcCurve::Nistp384 => Nid::SECP384R1,
            EcCurve::Nistp521 => Nid::SECP521R1,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            EcCurve::Nistp256 => NIST_P256_NAME,
            EcCurve::Nistp384 => NIST_P384_NAME,
            EcCurve::Nistp521 => NIST_P521_NAME,
        }
    }

    pub fn ident(&self) -> &'static str {
        match self {
            EcCurve::Nistp256 => "nistp256",
            EcCurve::Nistp384 => "nistp384",
            EcCurve::Nistp521 => "nistp521",
        }
    }
}

impl FromStr for EcCurve {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "nistp256" => Ok(EcCurve::Nistp256),
            "nistp384" => Ok(EcCurve::Nistp384),
            "nistp521" => Ok(EcCurve::Nistp521),
            _ => Err(Error::UnsupportedCurve),
        }
    }
}

impl TryInto<EcGroup> for EcCurve {
    type Error = Error;
    fn try_into(self) -> Result<EcGroup, Self::Error> {
        Ok(EcGroup::from_curve_name(self.nid())?)
    }
}

#[derive(Clone)]
pub struct EcDsaPublicKey {
    key: EcKey<Public>,
    curve: EcCurve,
}

//TODO: No Debug Implement for EcKey<Public>
impl fmt::Debug for EcDsaPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut dbg = f.debug_struct("EcDsaPublicKey");
        dbg.field("key", &"ECKEY".to_string());
        dbg.field("curve", &self.curve);
        dbg.finish()
    }
}

impl EcDsaPublicKey {
    pub fn new(group: &EcGroupRef, public_key: &EcPointRef) -> Result<Self, Error> {
        let curve = if let Some(nid) = group.curve_name() {
            match nid {
                Nid::X9_62_PRIME256V1 => EcCurve::Nistp256,
                Nid::SECP384R1 => EcCurve::Nistp384,
                Nid::SECP521R1 => EcCurve::Nistp521,
                _ => return Err(Error::InvalidFormat),
            }
        } else {
            return Err(Error::InvalidFormat);
        };

        Ok(Self {
            key: EcKey::from_public_key(group, public_key)?,
            curve: curve,
        })
    }
}

impl Key for EcDsaPublicKey {
    fn size(&self) -> usize {
        self.curve.size()
    }

    fn keytype(&self) -> &'static str {
        self.curve.name()
    }
}

impl PubKey for EcDsaPublicKey {
    fn blob(&self) -> Result<Vec<u8>, Error> {
        eckey_blob(self.curve, &self.key)
    }

    fn verify(&self, data: &[u8], sig: &[u8]) -> Result<bool, Error> {
        let pkey = PKey::from_ec_key(self.key.clone())?;
        let mut veri = Verifier::new(MessageDigest::sha1(), &pkey)?;
        veri.update(data)?;
        Ok(veri.verify(sig)?)
    }
}

impl PartialEq for EcDsaPublicKey {
    fn eq(&self, other: &Self) -> bool {
        let mut bn_ctx = BigNumContext::new().unwrap();
        //FIXME: rust-openssl doesn't provide a EC_GROUP_cmp() wrapper, so we temporarily use curve type instead.
        (self.curve == other.curve)
            && self
                .key
                .public_key()
                .eq(self.key.group(), other.key.public_key(), &mut bn_ctx)
                .unwrap()
    }
}

impl fmt::Display for EcDsaPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let body = base64::encode_config(&self.blob().unwrap(), base64::STANDARD);
        write!(f, "{} {}", self.curve.name(), &body)
    }
}

pub struct EcDsaKeyPair {
    key: EcKey<Private>,
    curve: EcCurve,
}

impl EcDsaKeyPair {
    pub fn clone_public_key(&self) -> Result<EcDsaPublicKey, Error> {
        Ok(EcDsaPublicKey::new(
            self.key.group(),
            self.key.public_key(),
        )?)
    }
}

impl Key for EcDsaKeyPair {
    fn size(&self) -> usize {
        self.curve.size()
    }

    fn keytype(&self) -> &'static str {
        self.curve.name()
    }
}

impl PubKey for EcDsaKeyPair {
    fn blob(&self) -> Result<Vec<u8>, Error> {
        eckey_blob(self.curve, &self.key)
    }

    fn verify(&self, data: &[u8], sig: &[u8]) -> Result<bool, Error> {
        self.clone_public_key()?.verify(data, sig)
    }
}

impl PrivKey for EcDsaKeyPair {
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, Error> {
        let pkey = PKey::from_ec_key(self.key.clone())?;
        let mut sign = Signer::new(MessageDigest::sha1(), &pkey)?;
        sign.update(data)?;
        Ok(sign.sign_to_vec()?)
    }
}

fn eckey_blob<T: HasPublic + HasParams>(
    curve: EcCurve,
    key: &EcKeyRef<T>,
) -> Result<Vec<u8>, Error> {
    let mut buf = Cursor::new(Vec::new());
    let mut bn_ctx = BigNumContext::new()?;

    buf.write_utf8(curve.name())?;
    buf.write_utf8(curve.ident())?;
    buf.write_string(&key.public_key().to_bytes(
        key.group(),
        PointConversionForm::UNCOMPRESSED,
        &mut bn_ctx,
    )?)?;

    Ok(buf.into_inner())
}

#[allow(non_upper_case_globals)]
#[cfg(test)]
mod test {
    use super::*;
    use openssl::bn::BigNumContext;
    use openssl::ec::EcPoint;

    const pub_str: &'static str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBKtcK82cEoqjiXyqPpyQAlkOQYs8LL5dDahPah5dqoaJfVHcKS5CJYBX0Ow+Dlj9xKtSQRCyJXOCEtJx+k4LUV0=";
    const ident: [u8; 0x08] = [0x6e, 0x69, 0x73, 0x74, 0x70, 0x32, 0x35, 0x36];
    const pub_key: [u8; 0x41] = [
        0x04, 0xab, 0x5c, 0x2b, 0xcd, 0x9c, 0x12, 0x8a, 0xa3, 0x89, 0x7c, 0xaa, 0x3e, 0x9c, 0x90,
        0x02, 0x59, 0x0e, 0x41, 0x8b, 0x3c, 0x2c, 0xbe, 0x5d, 0x0d, 0xa8, 0x4f, 0x6a, 0x1e, 0x5d,
        0xaa, 0x86, 0x89, 0x7d, 0x51, 0xdc, 0x29, 0x2e, 0x42, 0x25, 0x80, 0x57, 0xd0, 0xec, 0x3e,
        0x0e, 0x58, 0xfd, 0xc4, 0xab, 0x52, 0x41, 0x10, 0xb2, 0x25, 0x73, 0x82, 0x12, 0xd2, 0x71,
        0xfa, 0x4e, 0x0b, 0x51, 0x5d,
    ];

    fn get_test_pubkey() -> Result<EcDsaPublicKey, Error> {
        let ident_str = std::str::from_utf8(&ident).unwrap();
        let mut bn_ctx = BigNumContext::new()?;
        let group: EcGroup = EcCurve::from_str(ident_str)?.try_into()?;
        let point = EcPoint::from_bytes(&group, &pub_key, &mut bn_ctx)?;
        EcDsaPublicKey::new(&group, &point)
    }

    #[test]
    fn ecdsa_publickey_serialize() {
        let key = get_test_pubkey().unwrap();
        assert_eq!(key.to_string(), String::from(pub_str));
    }

    #[test]
    fn ecdsa_publickey_size() {
        let key = get_test_pubkey().unwrap();
        assert_eq!(key.size(), 256);
    }
}