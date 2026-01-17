//! Colored terminal output helpers
//!
//! When the `colored-output` feature is enabled, uses the colored crate
//! for terminal styling. Otherwise, outputs plain text.
//!
//! Security: Debug commands display warnings about sensitive data exposure.
//! Use `security::sanitize_for_log` for messages that may contain user prompt content.

// Re-export colored trait for conditional use in main.rs
#[cfg(feature = "colored-output")]
pub use colored::Colorize;

// Provide a no-op shim when colored is disabled
#[cfg(not(feature = "colored-output"))]
pub mod colorize_shim {
    /// Wrapper type that acts like colored::ColoredString but does nothing
    #[derive(Debug, Clone)]
    pub struct PlainString(pub String);

    impl std::fmt::Display for PlainString {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    // All color methods are no-ops, just return self
    impl PlainString {
        pub fn red(self) -> Self {
            self
        }
        pub fn green(self) -> Self {
            self
        }
        pub fn yellow(self) -> Self {
            self
        }
        pub fn blue(self) -> Self {
            self
        }
        pub fn cyan(self) -> Self {
            self
        }
        pub fn dimmed(self) -> Self {
            self
        }
        pub fn bold(self) -> Self {
            self
        }
        pub fn underline(self) -> Self {
            self
        }
    }

    /// No-op Colorize trait implementation for plain text output
    pub trait Colorize {
        fn to_plain(&self) -> PlainString;

        fn red(&self) -> PlainString {
            self.to_plain()
        }
        fn green(&self) -> PlainString {
            self.to_plain()
        }
        fn yellow(&self) -> PlainString {
            self.to_plain()
        }
        fn blue(&self) -> PlainString {
            self.to_plain()
        }
        fn cyan(&self) -> PlainString {
            self.to_plain()
        }
        fn dimmed(&self) -> PlainString {
            self.to_plain()
        }
        fn bold(&self) -> PlainString {
            self.to_plain()
        }
        fn underline(&self) -> PlainString {
            self.to_plain()
        }
    }

    impl Colorize for &str {
        fn to_plain(&self) -> PlainString {
            PlainString(self.to_string())
        }
    }

    impl Colorize for str {
        fn to_plain(&self) -> PlainString {
            PlainString(self.to_string())
        }
    }

    impl Colorize for String {
        fn to_plain(&self) -> PlainString {
            PlainString(self.clone())
        }
    }
}

#[cfg(not(feature = "colored-output"))]
pub use colorize_shim::Colorize;

pub fn print_error(msg: &str) {
    #[cfg(feature = "colored-output")]
    {
        use colored::Colorize as _;
        eprintln!("{} {}", "[cjk-token]".red(), msg);
    }

    #[cfg(not(feature = "colored-output"))]
    eprintln!("[cjk-token] {}", msg);
}

pub fn print_verbose(msg: &str, verbose: bool) {
    if verbose {
        #[cfg(feature = "colored-output")]
        {
            use colored::Colorize as _;
            eprintln!("{} {}", "[cjk-token]".dimmed(), msg);
        }

        #[cfg(not(feature = "colored-output"))]
        eprintln!("[cjk-token] {}", msg);
    }
}

/// Print a warning message about sensitive data exposure
pub fn print_sensitive_warning() {
    #[cfg(feature = "colored-output")]
    {
        use colored::Colorize as _;
        eprintln!(
            "{} {}",
            "[cjk-token]".yellow(),
            crate::security::SENSITIVE_DATA_WARNING
        );
    }

    #[cfg(not(feature = "colored-output"))]
    eprintln!("[cjk-token] {}", crate::security::SENSITIVE_DATA_WARNING);
}
