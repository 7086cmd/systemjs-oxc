use oxc::allocator::Allocator;
use oxc::ast::ast;
use oxc::semantic::{Scoping, SemanticBuilder};
use oxc::transformer::{EnvOptions, TransformOptions, Transformer};
use std::path::Path;

pub fn transform_to_es5<'a>(
    program: &mut ast::Program<'a>,
    allocator: &'a Allocator,
    source_path: &Path,
) -> Scoping {
    let ret = SemanticBuilder::new()
        // Estimate transformer will triple scopes, symbols, references
        .with_excess_capacity(2.0)
        .build(&program);
    let env = EnvOptions::from_target("es5").expect("Should be able to create EnvOptions for ES5");
    let options = TransformOptions {
        env,
        ..TransformOptions::default()
    };
    let transformer = Transformer::new(allocator, source_path, &options);
    let transformed = transformer.build_with_scoping(ret.semantic.into_scoping(), program);
    transformed.scoping
}
