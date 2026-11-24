//! Values kerfd writes as names: a refusal's code, where a proposal, a render or the
//! project file stands, what a render is asked for, and what staged a proposal.
//!
//! A kerfd and its clients may be of different builds, and what kerfd keeps on disk outlives
//! the build that wrote it. So each such value is an enum of the names this build knows, and
//! a name it does not know still reads: it is kept as it came, and written the same.

/// A name this build has no value for. Only a value's `from_name` makes one, and nothing
/// outside this crate can, so a name this build does know is always its own variant, and
/// two values are equal just when their names are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unnamed(String);

impl Unnamed {
    /// For [`named!`]'s `from_name` alone, which has looked for the name among its own first.
    pub(crate) fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// An enum of named values: each written once with its name, so a value cannot be given one
/// name going out and read by another coming in.
macro_rules! named {
    (
        $(#[$about:meta])*
        $vis:vis enum $Value:ident {
            $($(#[$note:meta])* $value:ident = $name:literal,)+
        }
    ) => {
        $(#[$about])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        $vis enum $Value {
            $($(#[$note])* $value,)+
            /// A name this build has no value for, as it came: a kerfd of another build
            /// wrote it, or kerfd passes it on as something else said it. Only
            /// [`Self::from_name`] makes one.
            Other($crate::named::Unnamed),
        }

        impl $Value {
            /// Its name, as it is written.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$value => $name,)+
                    Self::Other(name) => name.as_str(),
                }
            }

            /// The value a name stands for. For a name that comes from elsewhere (off the
            /// wire, or out of a file): a value said here is written as its variant, which
            /// the compiler checks.
            // Two values with one name would make the second arm unreachable.
            #[deny(unreachable_patterns)]
            pub fn from_name(name: &str) -> Self {
                match name {
                    $($name => Self::$value,)+
                    _ => Self::Other($crate::named::Unnamed::new(name)),
                }
            }
        }

        impl ::std::fmt::Display for $Value {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl ::serde::Serialize for $Value {
            fn serialize<S: ::serde::Serializer>(
                &self,
                serializer: S,
            ) -> ::std::result::Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $Value {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                <String as ::serde::Deserialize>::deserialize(deserializer)
                    .map(|name| Self::from_name(&name))
            }
        }
    };
}

pub(crate) use named;
