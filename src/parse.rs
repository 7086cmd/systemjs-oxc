use oxc::allocator::Allocator;
use oxc::ast::ast;
use oxc::parser::Parser;
use oxc::span::SourceType;

pub fn parse_program<'a>(source: &'a str, alloc: &'a Allocator) -> ast::Program<'a> {
    let parser = Parser::new(alloc, source, SourceType::mjs());
    parser.parse().program
}
