use trybuild::TestCases;

// Running the passing compile tests using `trybuild::pass` is slow
// This imports all the passing compile tests. If there are breaking changes,
// then the whole build will break
#[allow(dead_code)]
#[path = "compile/pass/mod.rs"]
mod compile_pass_tests;

#[rustversion::attr(not(stable), ignore)]
#[test]
fn compile() {
    let t = TestCases::new();
    t.compile_fail("tests/compile/compile_fail/*.rs");
}
