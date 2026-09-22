#[test]
fn the_handoff_pipe_cannot_be_squatted_or_used_to_impersonate() {
    // A clinic reception machine is a shared workstation, and what travels over
    // this pipe is a list of scan paths -- in dental work, patient identifiers.
    // The name alone is not a boundary: anything in the session can create
    // `\\.\pipe\<name>` first and wait for the real client to connect to it.
    let windows = include_str!("../single_instance/windows.rs");

    assert!(
        windows.contains("SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION"),
        "the client must cap the connection at identification level; at the \
         default impersonation level a hijacked listener can call \
         ImpersonateNamedPipeClient and act as this user"
    );
    assert!(
        windows.contains("FILE_FLAG_FIRST_PIPE_INSTANCE"),
        "the listener must fail when the name already exists rather than \
         becoming a second instance beside whoever claimed it"
    );
    assert!(
        windows.contains("D:P(A;;GA;;;{sid})"),
        "the listener's DACL must grant the current user and nobody else"
    );
    assert!(
        windows.contains("ConvertSidToStringSidW"),
        "the DACL needs the real SID, not a hash of it"
    );
    assert!(
        windows.contains("owner_only_security_descriptor"),
        "the security descriptor should be built in one named place"
    );

    // All of the above is reachable only when the SID lookup succeeds. Fall
    // back on failure and the pipe gets the default DACL under a fixed name:
    // the squattable, world-readable pipe this test is about, arrived at by a
    // silent downgrade rather than by an attack.
    assert!(
        !windows.contains("descriptor.unwrap_or_default()"),
        "a missing descriptor must refuse the pipe, not fall back to the \
         default DACL"
    );
    assert!(
        windows.contains(
            "bail!(\"refusing to create the single-instance pipe without an owner-only DACL\")"
        ),
        "the failure has to be a refusal, and it has to say so"
    );

    // FIRST_PIPE_INSTANCE makes a claimed name fail for good, so retrying it
    // every 50 ms wrote a warning per retry into a 50-line crash ring: the
    // whole recent-log section of the next crash report became one repeated
    // line, in under three seconds.
    assert!(
        windows.contains("const MAX_CONSECUTIVE_PIPE_FAILURES: u32"),
        "the listener must give up on a permanently claimed name"
    );
    // A listener that refuses the pipe is only half of it: the sender still
    // writes the scan paths to a name built from the same failed lookup.
    assert!(
        !windows.contains("String::from(\"default\")"),
        "a name that is not per-user must not be constructed at all"
    );
    for refusal in [
        "refusing to send an open request to a pipe name that is not per-user",
        "refusing to listen on a pipe name that is not per-user",
    ] {
        assert!(
            windows.contains(refusal),
            "both ends of the hand-off must refuse a shared name: {refusal}"
        );
    }
    assert!(
        windows.contains("hand-off continues"),
        "and it must say where hand-off went, because the disk fallback \
         listener carries it from there"
    );
}
