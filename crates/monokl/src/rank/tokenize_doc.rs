use crate::tokens::tokenize_source;
use crate::types::CodeBlock;

/// Returns the BM25 document tokens of a block: its code's tokens followed by its signature's, if it has one.
///
/// # Examples
///
/// ```
/// use camino::Utf8PathBuf;
/// use monokl::rank::tokenize_block;
/// use monokl::types::{CodeBlock, SymbolKind};
///
/// let block = CodeBlock {
///     file: Utf8PathBuf::from("src/a.ts"),
///     line_start: 1,
///     line_end: 1,
///     node_kind: SymbolKind::Function,
///     code: "return userName;".to_string(),
///     symbol_signature: Some("fn userName()".to_string()),
///     matched_lines: Vec::new(),
///     matched_keywords: Vec::new(),
/// };
/// assert_eq!(tokenize_block(&block), ["user", "name", "user", "name"]);
/// ```
pub fn tokenize_block(block: &CodeBlock) -> Vec<String> {
    let mut tokens = tokenize_source(&block.code);
    if let Some(signature) = &block.symbol_signature {
        tokens.extend(tokenize_source(signature));
    }
    tokens
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::*;
    use crate::types::SymbolKind;

    fn block(code: &str, symbol_signature: Option<&str>) -> CodeBlock {
        CodeBlock {
            file: Utf8PathBuf::from("src/a.ts"),
            line_start: 1,
            line_end: 3,
            node_kind: SymbolKind::Function,
            code: code.to_string(),
            symbol_signature: symbol_signature.map(ToString::to_string),
            matched_lines: Vec::new(),
            matched_keywords: Vec::new(),
        }
    }

    #[test]
    fn tokenize_block_appends_signature_tokens_after_code_tokens() {
        let block = block("const total = sumItems(cart);", Some("function sumItems(cart: Cart): number"));
        assert_eq!(
            tokenize_block(&block),
            ["total", "sum", "items", "cart", "function", "sum", "items", "cart", "cart", "number"]
        );
    }

    #[test]
    fn tokenize_block_without_signature_tokenizes_only_the_code() {
        assert_eq!(tokenize_block(&block("parseJson(raw)", None)), ["parse", "json", "raw"]);
    }

    #[test]
    fn tokenize_block_of_empty_code_and_no_signature_is_empty() {
        assert_eq!(tokenize_block(&block("", None)), Vec::<String>::new());
    }
}
