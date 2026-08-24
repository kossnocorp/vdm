use crate::prelude::*;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Expression, ImportExpression, TSImportEqualsDeclaration, TSImportType,
    TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;

pub fn javascript_dependencies(path: &Path, bytes: &[u8]) -> Result<Vec<String>> {
    let source_type = SourceType::from_path(path)
        .map_err(|error| Error::msg(error.to_string()))
        .with_context(|| format!("Failed to determine source type for {}", path.display()))?;
    let source = std::str::from_utf8(bytes)
        .with_context(|| format!("{} is not valid UTF-8", path.display()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    ensure!(
        parsed.diagnostics.is_empty(),
        "Failed to parse {}: {:?}",
        path.display(),
        parsed.diagnostics
    );

    let mut dependencies = BTreeSet::new();
    for specifier in parsed.module_record.requested_modules.keys() {
        if !specifier.is_empty() {
            dependencies.insert(specifier.to_string());
        }
    }

    let mut collector = DependencyCollector {
        dependencies: &mut dependencies,
    };
    collector.visit_program(&parsed.program);
    Ok(dependencies.into_iter().collect())
}

struct DependencyCollector<'a> {
    dependencies: &'a mut BTreeSet<String>,
}

impl DependencyCollector<'_> {
    fn add_expression(&mut self, expression: &Expression<'_>) {
        let value = match expression {
            Expression::StringLiteral(literal) => Some(literal.value.as_str()),
            Expression::TemplateLiteral(template) if template.is_no_substitution_template() => {
                Some(template.quasis[0].value.cooked.as_ref().map_or_else(
                    || template.quasis[0].value.raw.as_str(),
                    |value| value.as_str(),
                ))
            }
            _ => None,
        };
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            self.dependencies.insert(value.to_owned());
        }
    }
}

impl<'a> Visit<'a> for DependencyCollector<'_> {
    fn visit_import_expression(&mut self, expression: &ImportExpression<'a>) {
        self.add_expression(&expression.source);
        walk::walk_import_expression(self, expression);
    }

    fn visit_ts_import_type(&mut self, import: &TSImportType<'a>) {
        if !import.source.value.is_empty() {
            self.dependencies.insert(import.source.value.to_string());
        }
        walk::walk_ts_import_type(self, import);
    }

    fn visit_ts_import_equals_declaration(&mut self, import: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(reference) = &import.module_reference
            && !reference.expression.value.is_empty()
        {
            self.dependencies
                .insert(reference.expression.value.to_string());
        }
        walk::walk_ts_import_equals_declaration(self, import);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &call.callee
            && callee.name == "require"
            && call.arguments.len() == 1
            && let Some(argument) = call.arguments[0].as_expression()
        {
            self.add_expression(argument);
        }
        walk::walk_call_expression(self, call);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_static_javascript_and_typescript_dependencies() {
        let source = br#"
            import value from "./static.ts";
            export * from "./exported.ts";
            import type { Type } from "./type.ts";
            type Inline = import("./inline.ts").Type;
            const dynamic = import(`./dynamic.ts`);
            const commonjs = require("./commonjs.ts");
            const ignored = import("./" + name);
        "#;
        assert_eq!(
            javascript_dependencies(Path::new("file.ts"), source).unwrap(),
            vec![
                "./commonjs.ts",
                "./dynamic.ts",
                "./exported.ts",
                "./inline.ts",
                "./static.ts",
                "./type.ts",
            ]
        );
    }
}
