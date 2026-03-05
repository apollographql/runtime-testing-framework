use rtf_config::{
    StableSource,
    templating::{Field, Template, TemplateContext},
};
use rtf_derive::Template;
use std::collections::HashMap;

#[derive(Debug, Template)]
pub enum MyEnum {
    A(MyStruct),
    B(Bar),
    C(Field<String>),
    D,
}

#[derive(Debug, Template)]
pub struct MyStruct {
    pub foo: Field<String>,
    pub bar: Bar,
    #[template(skip)]
    pub baz: String,
}

#[derive(Debug, Template)]
pub struct Bar {
    pub inner: Field<u32>,
}

fn main() {
    let mut s = MyStruct {
        foo: Field::Pending("FOO".to_string()),
        bar: Bar {
            inner: Field::Resolved(42),
        },
        baz: "BAZ".to_string(),
    };

    println!("{:?}", s.required_variables());
    println!("{:?}", s);

    let mut vals = HashMap::new();
    vals.insert("FOO".to_string(), "a value for foo".into());

    s.try_template(
        &mut Vec::new(),
        &StableSource::TestPlan,
        &TemplateContext::new(
            vals,
            StableSource::TestPlan,
            Default::default(),
            Default::default(),
        ),
    )
    .unwrap();

    println!("{:?}", s);
}
