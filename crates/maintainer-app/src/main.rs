fn main() {}

#[cfg(test)]
mod tests {
    #[test]
    fn core_crate_is_linked() {
        assert_eq!(maintainer_core::PRODUCT_NAME, "Maintainer");
    }
}
