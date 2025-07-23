use oxc::ast::ast;
use oxc::codegen::Codegen;

pub fn generate_code<'a>(program: &ast::Program<'a>) -> String {
    let codegen = Codegen::new();
    let generated = codegen.build(program);
    generated.code
}
