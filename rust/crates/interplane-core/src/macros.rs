/// A string enum that keeps unknown values as `Unknown(String)`.
macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($var,)+
            /// A value this implementation does not know.
            Unknown(String),
        }
        impl $name {
            /// The wire string.
            pub fn as_str(&self) -> &str {
                match self { $(Self::$var => $s,)+ Self::Unknown(s) => s }
            }
            /// Parse a wire string; unknown strings become `Unknown`.
            pub fn parse(s: &str) -> Self {
                match s { $($s => Self::$var,)+ other => Self::Unknown(other.to_string()) }
            }
            /// True unless this is `Unknown`.
            pub fn is_known(&self) -> bool { !matches!(self, Self::Unknown(_)) }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(self.as_str()) }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <String as serde::Deserialize>::deserialize(d)?;
                Ok(Self::parse(&s))
            }
        }
    };
}

/// A unit type that serializes to a constant `kind` string and refuses any other.
macro_rules! const_kind {
    ($(#[$m:meta])* $name:ident, $lit:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name;
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str($lit) }
        }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <String as serde::Deserialize>::deserialize(d)?;
                if s == $lit { Ok($name) } else {
                    Err(serde::de::Error::custom(format!("expected kind {:?}, got {:?}", $lit, s)))
                }
            }
        }
    };
}
