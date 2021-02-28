#[tokio::test]
async fn test_parser() {
    use quill_common::location::SourceFileIdentifier;
    use quill_lexer::lex;
    use quill_source_file::ErrorEmitter;
    use quill_source_file::PackageFileSystem;
    use std::path::PathBuf;

    use quill_parser::parse;

    let fs = PackageFileSystem::new(PathBuf::from("tests"));
    let file_ident = SourceFileIdentifier {
        module: vec![].into(),
        file: "file".into(),
    };

    let lexed = lex(&fs, &file_ident).await;
    let parsed = lexed.bind(|lexed| parse(lexed, &file_ident));

    let mut error_emitter = ErrorEmitter::new(&fs);
    let parsed = error_emitter.consume_diagnostic(parsed);
    error_emitter.emit_all().await;

    // If the parse fails, the test will fail.
    let parsed = parsed.unwrap();

    println!("parsed: {:#?}", parsed);
}