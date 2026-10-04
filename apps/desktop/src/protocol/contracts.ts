/**
 * The typed contract surfaces this client speaks, with their versions. Sent in
 * `hello.contracts`; the daemon answers with its own in `hello_ok.contracts`.
 * Kept in step with `contract/<surface>.schema.json` by contracts.test.ts.
 */
export const CONTRACTS: Readonly<Record<string, number>> = { board: 1 };
