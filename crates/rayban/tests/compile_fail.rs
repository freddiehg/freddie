//! Compile-fail tests for the derive's own error messages. These assert the
//! macro rejects bad input with a clear message, not a downstream type error.

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
