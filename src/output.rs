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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "colored-output")]
    mod colored_feature_tests {
        use super::*;

        #[test]
        fn test_colored_error_output() {
            // Capture stderr to verify colored output
            let output = std::io::stderr();
            let _lock = output.lock();

            // This test mainly verifies that the function can be called without error
            // Actual color verification is difficult without capturing output
            print_error("Test error message");
        }

        #[test]
        fn test_colored_verbose_output_enabled() {
            // Test with verbose enabled
            print_verbose("Test verbose message", true);

            // Test with verbose disabled
            print_verbose("Test verbose message", false);
        }

        #[test]
        fn test_colored_sensitive_warning() {
            print_sensitive_warning();
        }
    }

    #[cfg(not(feature = "colored-output"))]
    mod colorize_shim_tests {
        use super::*;

        #[test]
        fn test_colorize_shim_plain_string_creation() {
            let plain = colorize_shim::PlainString("test".to_string());
            assert_eq!(plain.0, "test");
        }

        #[test]
        fn test_colorize_shim_display() {
            let plain = colorize_shim::PlainString("test".to_string());
            let result = format!("{}", plain);
            assert_eq!(result, "test");
        }

        #[test]
        fn test_colorize_shim_methods_return_self() {
            let plain = colorize_shim::PlainString("test".to_string());
            let result = plain
                .red()
                .green()
                .yellow()
                .blue()
                .cyan()
                .dimmed()
                .bold()
                .underline();
            assert_eq!(result.0, "test");
        }

        #[test]
        fn test_colorize_shim_trait_for_str() {
            use colorize_shim::Colorize;
            let result = "test".red();
            assert_eq!(result.0, "test");
        }

        #[test]
        fn test_colorize_shim_trait_for_string() {
            use colorize_shim::Colorize;
            let result = String::from("test").blue();
            assert_eq!(result.0, "test");
        }
    }

    // Tests for functions that work regardless of colored-output feature
    #[test]
    fn test_print_error() {
        // This test mainly verifies that the function can be called without error
        print_error("Test error message");
    }

    #[test]
    fn test_print_verbose_enabled() {
        print_verbose("Test verbose message", true);
    }

    #[test]
    fn test_print_verbose_disabled() {
        print_verbose("Test verbose message", false);
    }

    #[test]
    fn test_print_sensitive_warning() {
        print_sensitive_warning();
    }
}
