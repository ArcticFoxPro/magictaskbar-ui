macro_rules! define_app_errors {
    ($(
        $variant:ident($error_type:ty);
    )*) => {
        #[derive(Debug)]
        pub enum LibError {
            $(
                $variant($error_type),
            )*
        }

        $(
            impl From<$error_type> for LibError {
                fn from(err: $error_type) -> Self {
                    LibError::$variant(err)
                }
            }
        )*
    };
}

define_app_errors!(
    Custom(String);
    Io(std::io::Error);
    SerdeJson(serde_json::Error);
    SerdeYaml(serde_yaml::Error);
    Base64Decode(base64::DecodeError);
    Grass(Box<grass::Error>);
);

impl From<&str> for LibError {
    fn from(err: &str) -> Self {
        err.to_owned().into()
    }
}

impl std::fmt::Display for LibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

pub type Result<T, E = LibError> = std::result::Result<T, E>;
