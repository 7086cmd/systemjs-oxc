use oxc::ast_visit::VisitMut;
use std::path::Path;
mod codegen;
mod options;
mod parse;
mod transform;
mod transpiler;
mod visit;

fn main() {
    let transpiler_options = options::SystemJsTranspilerOptions::default();
    println!("Transpiler Options: {:#?}", transpiler_options);
    let path = Path::new("example.js");
    let source_text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} not found.\n{err}", path.display()));
    let allocator = oxc::allocator::Allocator::default();
    let mut program = parse::parse_program(source_text.as_str(), &allocator);
    transform::transform_to_es5(&mut program, &allocator, path);
    let mut transpiler = transpiler::SystemJsTranspiler::new(
        transpiler_options,
        &allocator,
    );
    transpiler.visit_program(&mut program);
    let codegen = codegen::generate_code(&program);
    println!("Transpiled Code:\n");
    println!("{}", codegen);
    // Write to `translated.js`
    let output_path = Path::new("translated.js");
    std::fs::write(output_path, codegen)
        .unwrap_or_else(|err| panic!("Failed to write to {}: {err}", output_path.display()));
}
