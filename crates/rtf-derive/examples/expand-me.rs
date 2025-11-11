use rtf_config::{
    providers::file::Source,
    templating::{Field, Template, TemplateValue},
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

    println!("{:?}", s.has_pending_fields());
    println!("{:?}", s.required_values());
    println!("{:?}", s);

    let mut vals = HashMap::new();
    vals.insert(
        "FOO".to_string(),
        TemplateValue {
            value: "a value for foo".into(),
            source: Source::local("/"),
        },
    );

    s.try_template(&mut Vec::new(), &vals).unwrap();
    println!("{:?}", s.has_pending_fields());
    println!("{:?}", s);
}
