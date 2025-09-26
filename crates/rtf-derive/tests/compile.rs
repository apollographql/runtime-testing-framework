use trybuild::TestCases;

#[rustversion::attr(not(stable), ignore)]
#[test]
fn compile() {
    let t = TestCases::new();
    t.pass("tests/compile/pass/*.rs");
    t.compile_fail("tests/compile/compile_fail/*.rs");
}
