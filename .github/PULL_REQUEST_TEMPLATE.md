## Summary

Describe the user-visible change and the problem it solves.

## Testing

- [ ] `./build.sh check` passes
- [ ] Added or updated regression tests
- [ ] Updated user documentation or the changelog when behavior changed

## Security and trust boundaries

- [ ] No new unbounded reads, child processes without timeouts, or shell
      interpolation of untrusted values
- [ ] IPC, plugin installation, file permissions, or native interop changes are
      explained below (if applicable)
- [ ] `cargo audit` was run when dependencies changed

## Attribution

- [ ] Any copied or adapted third-party code has an attribution comment and is
      listed in `THIRD_PARTY_NOTICES.md`
- [ ] External compatibility references (for example yabai or AeroSpace) are
      documented in the relevant module

## Related issues

Closes #
