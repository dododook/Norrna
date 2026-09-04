use crate::models::{Claims, JWT_SECRET, User};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

pub fn password_md5(password: &str) -> String {
    format!("{:x}", md5::compute(password.as_bytes()))
}

pub fn make_token(user: &User) -> anyhow::Result<String> {
    let exp = chrono::Utc::now().timestamp() as usize + 86400;
    let claims = Claims {
        sub: user.id.clone(),
        username: user.username.clone(),
        role: user.role.clone(),
        exp,
    };
    Ok(encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
    )?)
}

pub fn parse_token(token: &str) -> Option<Claims> {
    let mut validation = Validation::default();
    validation.validate_aud = false;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
        &validation,
    )
    .ok()
    .map(|d| d.claims)
}

pub fn cookie_header(token: &str) -> String {
    format!("token={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=86400")
}

pub fn clear_cookie() -> String {
    "token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".into()
}

pub fn token_from_cookie(header: Option<&str>) -> Option<String> {
    let h = header?;
    for part in h.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("token=") {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}
