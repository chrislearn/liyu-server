use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct Config {
    pub pool_max_size: u32,
    pub data_dir: PathBuf,
    pub admin_cookie_secure: bool,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        let pool_max_size =
            parse_pool_size(std::env::var("LIYU_DB_POOL_MAX_SIZE").ok().as_deref())?;
        let path = std::env::var("LIYU_DATA_DIR").unwrap_or_else(|_| "test-data".into());
        if path.trim().is_empty() {
            return Err("LIYU_DATA_DIR must not be empty".into());
        }
        let data_dir = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path);
        let admin_cookie_secure = match std::env::var("LIYU_ADMIN_COOKIE_SECURE").as_deref() {
            Ok("true") => true,
            Ok("false") | Err(_) => false,
            _ => return Err("LIYU_ADMIN_COOKIE_SECURE must be true or false".into()),
        };
        Ok(Self {
            pool_max_size,
            data_dir,
            admin_cookie_secure,
        })
    }

    pub fn prepare_storage(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.data_dir.join("avatars"))?;
        let products = self.data_dir.join("products");
        std::fs::create_dir_all(&products)?;
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/products");
        if fixtures != products {
            for entry in std::fs::read_dir(fixtures)? {
                let entry = entry?;
                let dest = products.join(entry.file_name());
                if entry.file_type()?.is_file() && !dest.exists() {
                    std::fs::copy(entry.path(), dest)?;
                }
            }
        }
        Ok(())
    }
}

fn parse_pool_size(value: Option<&str>) -> Result<u32, String> {
    match value {
        None => Ok(8),
        Some(v) => v
            .parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| "LIYU_DB_POOL_MAX_SIZE must be a positive integer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pool_size_rejects_invalid_configuration() {
        assert_eq!(parse_pool_size(None).unwrap(), 8);
        assert_eq!(parse_pool_size(Some("1")).unwrap(), 1);
        for value in ["0", "-1", "", "abc", "4294967296"] {
            assert!(parse_pool_size(Some(value)).is_err());
        }
    }
    #[test]
    fn custom_storage_is_seeded_without_overwriting_media() {
        let root = std::env::temp_dir().join(format!("liyu-storage-{}", uuid::Uuid::new_v4()));
        let config = Config {
            pool_max_size: 1,
            data_dir: root.clone(),
            admin_cookie_secure: false,
        };
        config.prepare_storage().unwrap();
        assert!(root.join("avatars").is_dir());
        let image = root.join("products/p00.png");
        assert!(image.is_file());
        std::fs::write(&image, b"keep custom image").unwrap();
        config.prepare_storage().unwrap();
        assert_eq!(std::fs::read(image).unwrap(), b"keep custom image");
        std::fs::remove_dir_all(root).unwrap();
    }
}
