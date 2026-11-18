// Included inside the existing tests module to reuse disposable FLAC fixtures.
fn field_rule(kind: &str, scope: &str, from: &str, to: &str) -> TransformRuleDto {
    serde_json::from_value(serde_json::json!({"kind":kind,"scope":scope,"from":from,"to":to})).unwrap()
}
fn field_group(rules: Vec<TransformRuleDto>) -> ActionGroupDto {
    ActionGroupDto { name:"Field recipe".into(), scope:"tags".into(), rules, note:String::new() }
}
#[test]
fn field_recipe_reads_staged_values_and_composes_removal_without_writing() {
    let dir=TempDir::new("field-recipe-staged");
    let path=dir.tagged_flac("one.flac","Saved","Song");
    let other=dir.tagged_flac("two.flac","Other","Other song");
    let mut app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let before=std::fs::read(&path).unwrap();
    let base=app.preview_tag_edits(&[
        TagEditDto {path:path.to_string_lossy().into(),field:"artist".into(),value:Some("AC/DC; Björk".into())},
        TagEditDto {path:other.to_string_lossy().into(),field:"title".into(),value:Some("Untouched staged".into())},
    ]).unwrap();
    let operations=vec![
        FieldOperation::Format {field:"custom:Venue".into(),pattern:"%artist% / 日本語".into()},
        FieldOperation::Format {field:"COMMENT".into(),pattern:"%custom:venue%".into()},
        FieldOperation::Remove {fields:vec!["ARTIST".into()]},
    ];
    let plan=app.preview_field_operations(std::slice::from_ref(&path),&operations,Some(&base)).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(),before,"preview must not write");
    let changed=plan.changes.iter().find(|c|c.path==path.to_string_lossy()).unwrap();
    assert_eq!(changed.tag_changes.iter().find(|f|f.field=="comment").unwrap().new.as_deref(),Some("AC/DC; Björk / 日本語"));
    assert!(changed.tag_changes.iter().any(|f|f.field=="artist"&&f.new.is_none()));
    assert!(plan.changes.iter().any(|c|c.path==other.to_string_lossy()&&c.tag_changes[0].new.as_deref()==Some("Untouched staged")));
    let batch=app.apply(&plan).unwrap();
    let saved=TagEngine::read(&path).unwrap();
    assert_eq!(saved.tags.get(&TagField::Comment).unwrap(),"AC/DC; Björk / 日本語");
    assert!(!saved.tags.contains_key(&TagField::Artist));
    app.undo(batch.id).unwrap();
    assert_eq!(TagEngine::read(&path).unwrap().tags.get(&TagField::Artist).unwrap(),"Saved");
    assert!(!TagEngine::read(&path).unwrap().tags.contains_key(&TagField::Comment));
}
#[test]
fn field_recipe_integrates_with_legacy_rules_and_locks() {
    let dir=TempDir::new("field-recipe-groups");
    let path=dir.tagged_flac("one.flac","Old","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let groups=vec![field_group(vec![
        field_rule("format","custom:Venue","","The Hall"),
        field_rule("case","custom:Venue","", ""),
        field_rule("format","comment","","%custom:venue% / %title%"),
        field_rule("remove_fields","tags","TITLE",""),
    ])];
    // Configure the ordinary case rule after constructing the context rules.
    let mut groups=groups; groups[0].rules[1].style="upper".into();
    app.set_locked_fields(&["title".into()]);
    let plan=app.preview_transform_groups(std::slice::from_ref(&path),&groups).unwrap();
    assert_eq!(plan.changes[0].tag_changes.iter().find(|f|f.field=="comment").unwrap().new.as_deref(),Some("THE HALL / Song"));
    assert!(!plan.changes[0].tag_changes.iter().any(|f|f.field=="title"));
    assert_eq!(plan.locked_skipped[0].field,"title");
}
#[test]
fn field_recipe_keep_only_preserves_artwork_and_explicit_cover_removal_undoes() {
    let dir=TempDir::new("field-recipe-cover");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    // A real tiny PNG exercises picture-byte preservation.
    let image=CoverArt { mime:"image/png".into(),data:std::sync::Arc::from(base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg==").unwrap()),kind:CoverKind::Back,description:"Booklet".into() };
    TagEngine::write_covers(&path,std::slice::from_ref(&image)).unwrap();
    let mut app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let before=TagEngine::read_covers(&path).unwrap();
    let keep=app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Keep {fields:vec!["title".into()]}],None).unwrap();
    assert!(keep.changes[0].cover_change.is_none());
    let combined=app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Remove {fields:vec!["PICTURE".into()]}],Some(&keep)).unwrap();
    let batch=app.apply(&combined).unwrap();
    assert!(TagEngine::read_covers(&path).unwrap().is_empty());
    app.undo(batch.id).unwrap();
    assert_eq!(TagEngine::read_covers(&path).unwrap(),before);
}
#[test]
fn field_recipe_preflight_cancellation_and_noops() {
    let dir=TempDir::new("field-recipe-preflight");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let invalid=FieldOperation::Format{field:"_FILENAME".into(),pattern:"x".into()};
    assert!(app.preview_field_operations(&[],&[invalid],None).is_err());
    assert!(app.preview_field_operations(&[],&[FieldOperation::Format{field:"title".into(),pattern:"%skip%".into()}],None).is_err());
    assert!(app.preview_field_operations_with_cancel(std::slice::from_ref(&path),&[],None,||true).is_err());
    let noop=app.preview_field_operations(&[path.clone(),path.clone()],&[FieldOperation::Format{field:"artist".into(),pattern:"%artist%".into()}],None).unwrap();
    assert!(noop.changes.is_empty());
    let clear=app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Format{field:"artist".into(),pattern:"%custom:Missing%".into()}],None).unwrap();
    assert_eq!(clear.changes[0].tag_changes[0].new,None);
}
#[test]
fn field_recipe_preserves_custom_spelling_and_rejects_case_collisions() {
    let dir=TempDir::new("field-recipe-spelling");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let base=app.preview_tag_edits(&[TagEditDto {path:path.to_string_lossy().into(),field:"custom:Venue".into(),value:Some("Hall".into())}]).unwrap();
    let plan=app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Format {field:"venue".into(),pattern:"Arena".into()}],Some(&base)).unwrap();
    assert_eq!(plan.changes[0].tag_changes.iter().find(|f|f.field=="custom:Venue").unwrap().new.as_deref(),Some("Arena"));
    assert!(!plan.changes[0].tag_changes.iter().any(|f|f.field=="custom:venue"));
}
#[test]
fn field_recipe_retains_structural_changes_and_original_old_values() {
    let dir=TempDir::new("field-recipe-rename");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let base=app.preview_rename("renamed",std::slice::from_ref(&path)).unwrap();
    let result=app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Format{field:"title".into(),pattern:"New".into()}],Some(&base)).unwrap();
    assert_eq!(result.changes[0].rename_to,base.changes[0].rename_to);
    assert_eq!(result.changes[0].tag_changes[0].old.as_deref(),Some("Song"));
}

#[test]
#[ignore = "manual performance measurement; creates 10,000 disposable tiny files"]
fn field_recipe_benchmark() {
    let dir=TempDir::new("field-recipe-benchmark");
    let source=dir.tagged_flac("source.flac","Artist / 日本語","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let operations=vec![
        FieldOperation::Format{field:"custom:Venue".into(),pattern:"Hall".into()},
        FieldOperation::Format{field:"comment".into(),pattern:"%custom:venue% / %artist%".into()},
        FieldOperation::Keep{fields:vec!["artist".into(),"title".into(),"comment".into()]},
    ];
    let paths=(0..10_000).map(|i| {let path=dir.0.join(format!("track-{i}.flac"));std::fs::copy(&source,&path).unwrap();path}).collect::<Vec<_>>();
    for count in [100,1_000,10_000] {
        let start=Instant::now();
        let first=app.preview_field_operations(&paths[..count],&operations,None).unwrap();
        let cold=start.elapsed();
        assert_eq!(first.changes.len(),count);
        let mut samples=Vec::new();
        for _ in 0..5 {
            let start=Instant::now();
            let plan=app.preview_field_operations(&paths[..count],&operations,None).unwrap();
            samples.push(start.elapsed().as_secs_f64()*1000.0);
            assert_eq!(plan.changes.len(),count);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!("FIELD_RECIPE files={count} first_ms={:.3} warm_median_ms={:.3} warm_max_ms={:.3}",cold.as_secs_f64()*1000.0,samples[2],samples[4]);
    }
}

#[test]
#[ignore = "manual independent verification; requires ffmpeg and ffprobe"]
fn field_recipe_independent_audio_roundtrip() {
    use std::process::Command;
    fn output(program:&str,args:&[&str])->Vec<u8> {
        let result=Command::new(program).args(args).output().unwrap();
        assert!(result.status.success(),"{program}: {}",String::from_utf8_lossy(&result.stderr));
        result.stdout
    }
    fn packets(path:&str)->Vec<u8> {output("ffmpeg",&["-v","error","-i",path,"-map","0:a:0","-c","copy","-f","hash","-hash","sha256","-"])}
    fn tags(path:&str)->serde_json::Value {
        let read: serde_json::Value = serde_json::from_slice(&output("ffprobe",&["-v","error","-show_entries","format_tags","-of","json",path])).unwrap();
        // Vorbis field names are case-insensitive and existing writers may
        // normalise their casing. Compare every independently read value.
        serde_json::Value::Object(read["format"]["tags"].as_object().unwrap().iter().map(|(key,value)|(key.to_ascii_lowercase(),value.clone())).collect())
    }
    let dir=TempDir::new("field-independent");
    let mut app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    for (ext,codec) in [("mp3","libmp3lame"),("flac","flac"),("m4a","aac")] {
        let path=dir.0.join(format!("fixture.{ext}"));
        let name=path.to_str().unwrap();
        output("ffmpeg",&["-v","error","-f","lavfi","-i","sine=frequency=440:duration=0.1","-c:a",codec,"-metadata","artist=Björk / 日本語","-metadata","title=Original","-y",name]);
        let before=tags(name);let audio=packets(name);let bytes=std::fs::read(&path).unwrap();
        let ops=vec![FieldOperation::Format{field:"comment".into(),pattern:"%artist% / live".into()},FieldOperation::Remove{fields:vec!["title".into()]}];
        let plan=app.preview_field_operations(std::slice::from_ref(&path),&ops,None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(),bytes);
        let batch=app.apply(&plan).unwrap();
        let after=tags(name);
        let saved=after.as_object().unwrap();
        assert!(saved.iter().any(|(key,value)|key.eq_ignore_ascii_case("comment")&&value=="Björk / 日本語 / live"),"{after}");
        assert!(!saved.keys().any(|key|key.eq_ignore_ascii_case("title")));
        assert_eq!(packets(name),audio,"encoded audio changed in {ext}");
        app.undo(batch.id).unwrap();
        assert_eq!(tags(name),before,"independent metadata differs after Undo in {ext}");
        assert_eq!(packets(name),audio);
        eprintln!("Independent {ext}: preview unchanged; Apply tags verified; encoded packets unchanged; Undo metadata restored");
    }
}

#[test]
fn field_recipe_refuses_stale_working_values_and_ambiguous_custom_targets() {
    let dir=TempDir::new("field-stale-ambiguous");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let base=app.preview_tag_edits(&[TagEditDto{path:path.to_string_lossy().into(),field:"title".into(),value:Some("Working".into())}]).unwrap();
    let mut external=TagEngine::read(&path).unwrap();external.tags.insert(TagField::Title,"External".into());TagEngine::write(&external).unwrap();
    assert!(matches!(app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Format{field:"comment".into(),pattern:"%title%".into()}],Some(&base)),Err(AppError::Plan(tagrex_core::plan::PlanError::Stale(_)))));
    let ambiguous=app.preview_tag_edits(&[
        TagEditDto{path:path.to_string_lossy().into(),field:"custom:Venue".into(),value:Some("One".into())},
        TagEditDto{path:path.to_string_lossy().into(),field:"custom:VENUE".into(),value:Some("Two".into())},
    ]).unwrap();
    assert!(app.preview_field_operations(std::slice::from_ref(&path),&[FieldOperation::Format{field:"venue".into(),pattern:"Three".into()}],Some(&ambiguous)).is_err());
}
#[test]
fn field_recipe_alias_selectors_and_explicit_custom_names_stay_distinct() {
    let dir=TempDir::new("field-selectors");
    let path=dir.tagged_flac("one.flac","Artist","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    let plan=app.preview_field_operations(std::slice::from_ref(&path),&[
        FieldOperation::Format{field:"custom:ARTIST".into(),pattern:"Different".into()},
        FieldOperation::Format{field:"DISCNUMBER".into(),pattern:"2".into()},
        FieldOperation::Keep{fields:vec!["custom:artist".into(),"disc".into()]},
    ],None).unwrap();
    let edits=&plan.changes[0].tag_changes;
    assert!(edits.iter().any(|e|e.field=="custom:ARTIST"&&e.new.as_deref()==Some("Different")));
    assert!(edits.iter().any(|e|e.field=="artist"&&e.new.is_none()));
    assert!(edits.iter().any(|e|e.field=="disc"&&e.new.as_deref()==Some("2")));
    let mixed=vec![field_group(vec![field_rule("format","title","","New"),field_rule("case","filename","","")])];
    assert!(validate_field_groups(&mixed).is_err());
}

#[test]
fn field_recipe_locked_intermediate_values_never_feed_later_steps() {
    let dir=TempDir::new("field-lock-input");
    let path=dir.tagged_flac("one.flac","Saved","Song");
    let app=App::open(&dir.0,&dir.0.join("journal.sqlite")).unwrap();
    app.set_locked_fields(&["artist".into()]);
    let plan=app.preview_field_operations(std::slice::from_ref(&path),&[
        FieldOperation::Format{field:"artist".into(),pattern:"Blocked".into()},
        FieldOperation::Format{field:"comment".into(),pattern:"%artist%".into()},
        FieldOperation::Remove{fields:vec!["artist".into()]},
    ],None).unwrap();
    assert_eq!(plan.changes[0].tag_changes.iter().find(|f|f.field=="comment").unwrap().new.as_deref(),Some("Saved"));
    assert!(!plan.changes[0].tag_changes.iter().any(|f|f.field=="artist"));
    assert_eq!(plan.locked_skipped[0].field,"artist");
}
