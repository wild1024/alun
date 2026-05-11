//! alun-utils 功能测试
//!
//! 覆盖：Crypto、Sid、Mask、Valid、Date、StrExt、Export、Web

#[cfg(test)]
mod tests {
    use alun_utils::crypto::Crypto;
    use alun_utils::sid::Sid;
    use alun_utils::mask::Mask;
    use alun_utils::valid::Valid;
    use alun_utils::date::Date;
    use alun_utils::str::{
        StrExt, sanitize_filename, parse_json_value, format_file_size,
        clean_string_param, clean_email, clean_password,
        InputCleaner, generate_invite_code, generate_random_digits,
        generate_random_alphanum,
    };
    use alun_utils::export::{Export, Import};
    use alun_utils::web::{WebExt, is_private_ip};

    // ──── Crypto ──────────────────────────────────────

    #[test]
    fn test_crypto_sha256() {
        let hash = Crypto::sha256("alun");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_crypto_sha256_deterministic() {
        let hash1 = Crypto::sha256("test");
        let hash2 = Crypto::sha256("test");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_crypto_sha256_different_inputs() {
        let hash1 = Crypto::sha256("hello");
        let hash2 = Crypto::sha256("world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_crypto_hmac() {
        let key = b"my-secret-key-123456789012";
        let sig = Crypto::hmac_sha256(key, "message");
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn test_crypto_aes_encrypt_decrypt_valid_key() {
        let key = vec![0u8; 32];
        let (cipher, nonce) = Crypto::aes_encrypt(&key, "secret-data").unwrap();
        let plain = Crypto::aes_decrypt(&key, &cipher, &nonce).unwrap();
        assert_eq!(plain, "secret-data");
    }

    #[test]
    fn test_crypto_aes_encrypt_decrypt_empty() {
        let key = vec![0u8; 32];
        let (cipher, nonce) = Crypto::aes_encrypt(&key, "").unwrap();
        let plain = Crypto::aes_decrypt(&key, &cipher, &nonce).unwrap();
        assert_eq!(plain, "");
    }

    #[test]
    fn test_crypto_aes_encrypt_decrypt_unicode() {
        let key = vec![1u8; 32];
        let data = "中文测试 🎉";
        let (cipher, nonce) = Crypto::aes_encrypt(&key, data).unwrap();
        let plain = Crypto::aes_decrypt(&key, &cipher, &nonce).unwrap();
        assert_eq!(plain, data);
    }

    #[test]
    fn test_crypto_aes_invalid_key_size() {
        let short_key = vec![0u8; 16];
        assert!(Crypto::aes_encrypt(&short_key, "data").is_none());
    }

    #[test]
    fn test_crypto_aes_decrypt_invalid_key() {
        let key32 = vec![0u8; 32];
        let wrong_key = vec![1u8; 32];
        let (cipher, nonce) = Crypto::aes_encrypt(&key32, "secret").unwrap();
        assert!(Crypto::aes_decrypt(&wrong_key, &cipher, &nonce).is_none());
    }

    #[test]
    fn test_crypto_aes_decrypt_wrong_nonce() {
        let key = vec![0u8; 32];
        let (cipher, _) = Crypto::aes_encrypt(&key, "secret").unwrap();
        let wrong_nonce = Crypto::base64_url_encode(&[0u8; 12]);
        assert!(Crypto::aes_decrypt(&key, &cipher, &wrong_nonce).is_none());
    }

    #[test]
    fn test_crypto_password_hash_verify() {
        let hash = Crypto::hash_password("StrongP@ss1").unwrap();
        assert!(Crypto::verify_password("StrongP@ss1", &hash).unwrap());
        assert!(!Crypto::verify_password("WrongPass1", &hash).unwrap());
    }

    #[test]
    fn test_crypto_password_different_hashes() {
        let hash1 = Crypto::hash_password("Pass1").unwrap();
        let hash2 = Crypto::hash_password("Pass1").unwrap();
        assert_ne!(hash1, hash2);
        assert!(Crypto::verify_password("Pass1", &hash1).unwrap());
        assert!(Crypto::verify_password("Pass1", &hash2).unwrap());
    }

    #[test]
    fn test_crypto_base64_url_encode_decode() {
        let original = b"hello world this is base64 url test";
        let encoded = Crypto::base64_url_encode(original);
        let decoded = Crypto::base64_url_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_crypto_base64_url_no_padding() {
        let encoded = Crypto::base64_url_encode(b"test");
        assert!(!encoded.ends_with('='));
    }

    #[test]
    fn test_crypto_random_key() {
        let key1 = Crypto::random_key();
        let key2 = Crypto::random_key();
        assert_eq!(key1.len(), 32);
        assert_eq!(key2.len(), 32);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_crypto_random_token() {
        let token = Crypto::random_token(16);
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ──── Sid ─────────────────────────────────────────

    #[test]
    fn test_sid_uuid() {
        let id = Sid::uuid();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sid_short() {
        let id = Sid::short();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sid_tiny() {
        let id = Sid::tiny();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sid_tsid() {
        let id = Sid::tsid();
        assert_eq!(id.len(), 20);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sid_uuid7() {
        let id = Sid::uuid7();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sid_uniqueness() {
        let mut ids = std::collections::HashSet::new();
        for _ in 0..100 {
            ids.insert(Sid::uuid());
        }
        assert_eq!(ids.len(), 100);
    }

    // ──── Mask ────────────────────────────────────────

    #[test]
    fn test_mask_mobile() {
        assert_eq!(Mask::mobile("13812345678"), "138****5678");
    }

    #[test]
    fn test_mask_mobile_short() {
        assert_eq!(Mask::mobile("123"), "123");
    }

    #[test]
    fn test_mask_email() {
        assert_eq!(Mask::email("alice@mail.com"), "a***@mail.com");
    }

    #[test]
    fn test_mask_email_short_prefix() {
        assert_eq!(Mask::email("ab@mail.com"), "*@mail.com");
    }

    #[test]
    fn test_mask_id_card() {
        assert_eq!(Mask::id_card("110101199001011234"), "1101****1234");
    }

    #[test]
    fn test_mask_bank_card() {
        assert_eq!(Mask::bank_card("6222021234567890"), "6222 **** 7890");
    }

    #[test]
    fn test_mask_bank_card_short() {
        assert_eq!(Mask::bank_card("12345"), "12345");
    }

    #[test]
    fn test_mask_name() {
        assert_eq!(Mask::name("张三"), "张*");
    }

    #[test]
    fn test_mask_name_single_char() {
        assert_eq!(Mask::name("李"), "李");
    }

    #[test]
    fn test_mask_name_three_chars() {
        assert_eq!(Mask::name("王小明"), "王**");
    }

    // ──── Valid ───────────────────────────────────────

    #[test]
    fn test_valid_email() {
        assert!(Valid::is_email("test@example.com"));
        assert!(Valid::is_email("a.b@mail.co.jp"));
        assert!(!Valid::is_email("not-email"));
        assert!(!Valid::is_email("@no-local.com"));
        assert!(!Valid::is_email(""));
    }

    #[test]
    fn test_valid_mobile() {
        assert!(Valid::is_mobile("13812345678"));
        assert!(Valid::is_mobile("19987654321"));
        assert!(!Valid::is_mobile("12345678901"));
        assert!(!Valid::is_mobile("1381234567"));
        assert!(!Valid::is_mobile(""));
    }

    #[test]
    fn test_valid_url() {
        assert!(Valid::is_url("https://example.com"));
        assert!(Valid::is_url("http://localhost:3000/api"));
        assert!(!Valid::is_url("not-a-url"));
        assert!(!Valid::is_url(""));
    }

    #[test]
    fn test_valid_digits() {
        assert!(Valid::is_digits("123456"));
        assert!(Valid::is_digits("0"));
        assert!(!Valid::is_digits("12a34"));
        assert!(!Valid::is_digits(""));
        assert!(!Valid::is_digits("12.34"));
    }

    #[test]
    fn test_valid_alphanumeric() {
        assert!(Valid::is_alphanumeric("abc123"));
        assert!(Valid::is_alphanumeric("ABC"));
        assert!(!Valid::is_alphanumeric("abc-123"));
        assert!(!Valid::is_alphanumeric(""));
        assert!(!Valid::is_alphanumeric("abc 123"));
    }

    #[test]
    fn test_valid_len_between() {
        assert!(Valid::len_between("abc", 1, 10));
        assert!(Valid::len_between("abc", 3, 3));
        assert!(!Valid::len_between("ab", 3, 10));
        assert!(!Valid::len_between("abcdef", 1, 3));
    }

    #[test]
    fn test_valid_len_between_unicode() {
        assert!(Valid::len_between("中文测试", 2, 10));
        assert!(!Valid::len_between("中文", 3, 10));
    }

    #[test]
    fn test_valid_strong_password() {
        assert!(Valid::is_strong_password("Abcdefg1"));
        assert!(Valid::is_strong_password("MyP@ssw0rd"));
        assert!(!Valid::is_strong_password("short1A"));
        assert!(!Valid::is_strong_password("nouppercase1"));
        assert!(!Valid::is_strong_password("NOLOWERC1"));
        assert!(!Valid::is_strong_password("NoDigits"));
    }

    #[test]
    fn test_valid_ipv4() {
        assert!(Valid::is_ipv4("192.168.1.1"));
        assert!(Valid::is_ipv4("127.0.0.1"));
        assert!(!Valid::is_ipv4("256.0.0.1"));
        assert!(!Valid::is_ipv4("not-ip"));
        assert!(!Valid::is_ipv4(""));
    }

    // ──── Date ────────────────────────────────────────

    #[test]
    fn test_date_now() {
        let now = Date::now();
        let ts = now.timestamp();
        assert!(ts > 1700000000);
    }

    #[test]
    fn test_date_fmt() {
        let dt = Date::from_timestamp(1710000000);
        let fmt1 = Date::fmt(&dt, "%Y-%m-%d");
        assert!(fmt1.starts_with("2024"));
    }

    #[test]
    fn test_date_from_timestamp() {
        let dt = Date::from_timestamp(1710000000);
        assert_eq!(dt.timestamp(), 1710000000);
    }

    #[test]
    fn test_date_begin_of_day() {
        let dt = Date::from_timestamp(1710003600);
        let bod = Date::begin_of_day(&dt);
        assert_eq!(bod.format("%H:%M:%S").to_string(), "00:00:00");
    }

    #[test]
    fn test_date_end_of_day() {
        let dt = Date::from_timestamp(1710003600);
        let eod = Date::end_of_day(&dt);
        assert_eq!(eod.format("%H:%M:%S").to_string(), "23:59:59");
    }

    #[test]
    fn test_date_relative_now() {
        let now_ts = chrono::Utc::now().timestamp();
        let desc = Date::relative(now_ts);
        assert!(desc.contains("秒前") || desc.contains("分钟前"));
    }

    #[test]
    fn test_date_relative_hours_ago() {
        let ts = chrono::Utc::now().timestamp() - 7200;
        let desc = Date::relative(ts);
        assert!(desc.contains("小时前"));
    }

    #[test]
    fn test_date_relative_days_ago() {
        let ts = chrono::Utc::now().timestamp() - 86400 * 3;
        let desc = Date::relative(ts);
        assert!(desc.contains("天前"));
    }

    // ──── StrExt ──────────────────────────────────────

    #[test]
    fn test_str_ext_is_blank() {
        assert!("   ".is_blank());
        assert!("\t\n".is_blank());
        assert!(!"hello".is_blank());
    }

    #[test]
    fn test_str_ext_has_text() {
        assert!("abc".has_text());
        assert!(!"   ".has_text());
    }

    #[test]
    fn test_str_ext_to_camel() {
        assert_eq!("user_name".to_camel(), "userName");
        assert_eq!("hello_world_test".to_camel(), "helloWorldTest");
        assert_eq!("single".to_camel(), "single");
    }

    #[test]
    fn test_str_ext_to_snake() {
        assert_eq!("UserName".to_snake(), "user_name");
        assert_eq!("HelloWorld".to_snake(), "hello_world");
        assert_eq!("single".to_snake(), "single");
    }

    #[test]
    fn test_str_ext_truncate() {
        assert_eq!("hello world".truncate(8), "hello wo...");
        assert_eq!("short".truncate(10), "short");
        assert_eq!("exact".truncate(5), "exact");
    }

    #[test]
    fn test_str_ext_random() {
        let s1: String = <str as StrExt>::random(8);
        let s2: String = <str as StrExt>::random(8);
        assert_eq!(s1.len(), 8);
        assert_eq!(s2.len(), 8);
        assert_ne!(s1, s2);
    }

    // ──── sanitize_filename ───────────────────────────

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("my file.txt"), "my_file.txt");
        assert_eq!(sanitize_filename("hello/world"), "hello_world");
        assert_eq!(sanitize_filename("valid-file.txt"), "valid-file.txt");
    }

    // ──── parse_json_value ────────────────────────────

    #[test]
    fn test_parse_json_value_valid() {
        let val = parse_json_value(r#"{"key":"value"}"#).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn test_parse_json_value_invalid() {
        assert!(parse_json_value("{invalid").is_err());
    }

    // ──── format_file_size ────────────────────────────

    #[test]
    fn test_format_file_size_zero() {
        assert_eq!(format_file_size(0), "0 B");
    }

    #[test]
    fn test_format_file_size_bytes() {
        let s = format_file_size(500);
        assert!(s.contains("B"));
    }

    #[test]
    fn test_format_file_size_kb() {
        let s = format_file_size(1024);
        assert!(s.contains("KB"));
    }

    #[test]
    fn test_format_file_size_mb() {
        let s = format_file_size(1_500_000);
        assert!(s.contains("MB"));
    }

    #[test]
    fn test_format_file_size_gb() {
        let s = format_file_size(2_000_000_000);
        assert!(s.contains("GB"));
    }

    // ──── clean_string ────────────────────────────────

    #[test]
    fn test_clean_string_param() {
        assert_eq!(clean_string_param("  hello  "), "hello");
        assert_eq!(clean_string_param("no-space"), "no-space");
    }

    #[test]
    fn test_clean_email() {
        assert_eq!(clean_email("  Alice@Mail.COM  "), "alice@mail.com");
    }

    #[test]
    fn test_clean_password() {
        assert_eq!(clean_password("  pass  word  "), "pass  word");
    }

    // ──── InputCleaner ────────────────────────────────

    #[test]
    fn test_input_cleaner_register() {
        let (email, password, nickname) = InputCleaner::clean_register_input(
            " Alice@Mail.COM ",
            " Pass123 ",
            "  John  ",
        );
        assert_eq!(email, "alice@mail.com");
        assert_eq!(password, "Pass123");
        assert_eq!(nickname, "John");
    }

    #[test]
    fn test_input_cleaner_login() {
        let (email, password) = InputCleaner::clean_login_input(
            " BOB@Example.Com   ",
            "   secret  ",
        );
        assert_eq!(email, "bob@example.com");
        assert_eq!(password, "secret");
    }

    // ──── generate_invite_code ────────────────────────

    #[test]
    fn test_generate_invite_code() {
        let code = generate_invite_code();
        assert_eq!(code.len(), 12);
        assert!(code.chars().all(char::is_alphanumeric));
    }

    // ──── generate_random_digits ──────────────────────

    #[test]
    fn test_generate_random_digits() {
        let s = generate_random_digits(6);
        assert_eq!(s.len(), 6);
        assert!(!s.contains('0'));
        assert!(s.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_generate_random_digits_zero_len() {
        assert_eq!(generate_random_digits(0), "");
    }

    // ──── generate_random_alphanum ────────────────────

    #[test]
    fn test_generate_random_alphanum() {
        let s = generate_random_alphanum(8);
        assert_eq!(s.len(), 8);
        assert!(!s.chars().any(|c| c == '0' || c == 'O' || c == 'I' || c == 'l'));
    }

    #[test]
    fn test_generate_random_alphanum_empty() {
        assert_eq!(generate_random_alphanum(0), "");
    }

    // ──── Export / Import ─────────────────────────────

    #[test]
    fn test_export_to_csv() {
        let mut row1 = std::collections::HashMap::new();
        row1.insert("id".into(), "1".into());
        row1.insert("name".into(), "Alice".into());

        let mut row2 = std::collections::HashMap::new();
        row2.insert("id".into(), "2".into());
        row2.insert("name".into(), "Bob".into());

        let csv = Export::to_csv(&["id", "name"], &[row1, row2]).unwrap();
        assert!(csv.contains("id,name"));
        assert!(csv.contains("1,Alice"));
        assert!(csv.contains("2,Bob"));
    }

    #[test]
    fn test_export_to_json() {
        #[derive(serde::Serialize, PartialEq, Debug)]
        struct Item { id: i64, name: String }

        let items = vec![
            Item { id: 1, name: "A".into() },
            Item { id: 2, name: "B".into() },
        ];
        let json = Export::to_json(&items).unwrap();
        let result: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["id"], 1);
        assert_eq!(result[0]["name"], "A");
    }

    #[test]
    fn test_export_to_xlsx() {
        let mut row = std::collections::HashMap::new();
        row.insert("id".into(), "1".into());
        row.insert("name".into(), "Test".into());

        let data = Export::to_xlsx(&["id", "name"], &[row]).unwrap();
        assert_eq!(&data[..3], &[0xEFu8, 0xBB, 0xBF]); // BOM
        assert!(data.len() > 3);
    }

    #[test]
    fn test_import_from_csv() {
        let csv = "id,name\n1,Alice\n2,Bob\n";
        let rows = Import::from_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("id").unwrap(), "1");
        assert_eq!(rows[0].get("name").unwrap(), "Alice");
        assert_eq!(rows[1].get("id").unwrap(), "2");
        assert_eq!(rows[1].get("name").unwrap(), "Bob");
    }

    #[test]
    fn test_import_from_json() {
        #[derive(serde::Deserialize, PartialEq, Debug)]
        struct Item { id: i64, name: String }

        let json = r#"[{"id":1,"name":"A"},{"id":2,"name":"B"}]"#;
        let items: Vec<Item> = Import::from_json(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], Item { id: 1, name: "A".into() });
    }

    #[test]
    fn test_import_from_csv_empty() {
        let rows = Import::from_csv("").unwrap();
        assert!(rows.is_empty());
    }

    // ──── Web ─────────────────────────────────────────

    #[test]
    fn test_web_ext_domain() {
        assert_eq!(WebExt::domain("https://example.com/path"), Some("example.com".into()));
        assert_eq!(WebExt::domain("http://localhost:3000"), Some("localhost".into()));
        assert_eq!(WebExt::domain("invalid"), None);
    }

    #[test]
    fn test_web_ext_path() {
        assert_eq!(WebExt::path("https://example.com/api/users"), Some("/api/users".into()));
        assert_eq!(WebExt::path("https://example.com/"), Some("/".into()));
        assert_eq!(WebExt::path("invalid"), None);
    }

    #[test]
    fn test_web_ext_real_ip() {
        let headers = vec![("X-Forwarded-For".into(), "10.0.0.1, 8.8.8.8".into())];
        assert_eq!(WebExt::real_ip(&headers, "127.0.0.1:8080"), "10.0.0.1");

        let headers2 = vec![("X-Real-Ip".into(), "192.168.1.1".into())];
        assert_eq!(WebExt::real_ip(&headers2, "10.0.0.1:8080"), "192.168.1.1");
    }

    #[test]
    fn test_web_ext_real_ip_fallback() {
        let headers: Vec<(String, String)> = vec![];
        assert_eq!(WebExt::real_ip(&headers, "8.8.8.8:443"), "8.8.8.8");
    }

    #[test]
    fn test_web_ext_build_query() {
        let q = WebExt::build_query(&[("key1", "val1"), ("key2", "val 2")]);
        assert!(q.starts_with('?'));
        assert!(q.contains("key1=val1"));
    }

    #[test]
    fn test_web_ext_build_query_empty() {
        assert_eq!(WebExt::build_query(&[]), "");
    }

    #[test]
    fn test_is_private_ip() {
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("192.168.1.1"));
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("::1"));
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("not-ip"));
    }
}