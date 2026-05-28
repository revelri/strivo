/**
 * Minimal ES256 JWT signer using Web Crypto.
 *
 * We intentionally don't pull `jose` — Workers ship a complete subtle
 * crypto, and a 60-line implementation has fewer moving parts than a
 * dep. The client (strivo-core/licence/cache) only needs to verify;
 * verification can use any compliant lib.
 */

interface ImportedKey {
  pem: string;
  key: CryptoKey;
}

let cached: ImportedKey | null = null;

async function importPrivateKey(pem: string): Promise<CryptoKey> {
  // Reuse the imported key across requests when the PEM doesn't change.
  // Workers reuse the global isolate per region, so this caches across
  // many requests without leaking between deployments.
  if (cached && cached.pem === pem) return cached.key;
  const b64 = pem
    .replace(/-----BEGIN EC PRIVATE KEY-----/, "")
    .replace(/-----END EC PRIVATE KEY-----/, "")
    .replace(/-----BEGIN PRIVATE KEY-----/, "")
    .replace(/-----END PRIVATE KEY-----/, "")
    .replace(/\s+/g, "");
  const der = base64ToBytes(b64);
  // openssl `EC PRIVATE KEY` is SEC1, but Web Crypto wants PKCS8.
  // We try PKCS8 first (modern openssl `genpkey -algorithm EC`)
  // then fall back to a hand-wrapped PKCS8 for SEC1.
  let key: CryptoKey;
  try {
    key = await crypto.subtle.importKey(
      "pkcs8",
      der,
      { name: "ECDSA", namedCurve: "P-256" },
      false,
      ["sign"],
    );
  } catch {
    key = await crypto.subtle.importKey(
      "pkcs8",
      sec1ToPkcs8(der),
      { name: "ECDSA", namedCurve: "P-256" },
      false,
      ["sign"],
    );
  }
  cached = { pem, key };
  return key;
}

/** Sign a JWT with ES256. `payload` is the claims object. */
export async function signEs256(
  pem: string,
  payload: Record<string, unknown>,
): Promise<string> {
  const header = { alg: "ES256", typ: "JWT" };
  const enc = (o: object) => base64urlEncode(new TextEncoder().encode(JSON.stringify(o)));
  const signingInput = `${enc(header)}.${enc(payload)}`;
  const key = await importPrivateKey(pem);
  const sigDer = new Uint8Array(
    await crypto.subtle.sign(
      { name: "ECDSA", hash: "SHA-256" },
      key,
      new TextEncoder().encode(signingInput),
    ),
  );
  // Web Crypto returns raw r||s for ECDSA — exactly what JWS wants.
  return `${signingInput}.${base64urlEncode(sigDer)}`;
}

/**
 * Wrap a SEC1 EC private key in a minimal PKCS8 envelope. openssl
 * `ecparam -genkey` emits SEC1; modern best practice is `genpkey`.
 * Worker callers shouldn't have to know the difference.
 */
function sec1ToPkcs8(sec1: Uint8Array): Uint8Array {
  // PKCS8 PrivateKeyInfo:
  //   SEQUENCE {
  //     INTEGER 0,                              -- version
  //     SEQUENCE {                              -- algorithm
  //       OID 1.2.840.10045.2.1,                --   id-ecPublicKey
  //       OID 1.2.840.10045.3.1.7               --   prime256v1
  //     },
  //     OCTET STRING { <SEC1 bytes> }
  //   }
  const algorithm = new Uint8Array([
    0x30, 0x13, // SEQ
    0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, // ecPublicKey
    0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, // prime256v1
  ]);
  const octetString = new Uint8Array([0x04, sec1.length, ...sec1]);
  const inner = new Uint8Array([
    0x02, 0x01, 0x00, // version=0
    ...algorithm,
    ...octetString,
  ]);
  return new Uint8Array([0x30, 0x82, (inner.length >> 8) & 0xff, inner.length & 0xff, ...inner]);
}

function base64urlEncode(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Seconds since epoch. */
export const nowSecs = () => Math.floor(Date.now() / 1000);
