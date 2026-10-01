use slimpack::schema::{Attr, FieldSpec, StructSpec, Type};
use slimpack::SlimPack;
use slimpack_derive::SlimPack;

#[derive(SlimPack)]
struct Example {
    #[slim(range = 0..=10)]
    value: u8,
    #[slim(bits = 3)]
    flag: bool,
}

#[test]
fn derive_keeps_the_struct_intact() {
    let sample = Example { value: 5, flag: true };
    assert_eq!(sample.value, 5);
    assert!(sample.flag);
}

#[test]
fn derive_builds_a_schema_from_attributes() {
    let spec = Example::slim_schema();
    assert_eq!(
        spec,
        StructSpec::new(
            "Example",
            vec![
                FieldSpec::new("value", Type::U8).with(Attr::Range { min: 0, max: 10 }),
                FieldSpec::new("flag", Type::Bool).with(Attr::Bits(3)),
            ],
        )
    );
}
