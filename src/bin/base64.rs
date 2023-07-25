use base64::{engine::general_purpose, Engine as _};
use rand::Rng;

fn main() {
    let mut key: [u8; 32] = [0; 32];
    let mut nonce: [u8; 12] = [0; 12];

    let mut rng = rand::thread_rng();
    (0..32).into_iter().for_each(|i| key[i] = rng.gen());
    (0..12).into_iter().for_each(|i| nonce[i] = rng.gen());

    println!("key: {}", general_purpose::STANDARD.encode(key).as_str());
    println!(
        "nonce: {}",
        general_purpose::STANDARD.encode(nonce).as_str()
    );
}
