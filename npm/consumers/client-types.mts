import type { StoredSchema } from '../src/client.js';

type Assert<T extends true> = T;
type Equal<Left, Right> = (<T>() => T extends Left ? 1 : 2) extends (<T>() => T extends Right ? 1 : 2) ? true : false;
type RequiredKeys<T> = { [Key in keyof T]-?: {} extends Pick<T, Key> ? never : Key }[keyof T];

// Keep the public response fields required; this contract caught the optional
// updatedAt declaration that made the package's own consumer check fail.
type StoredSchemaFieldsAreRequired = Assert<Equal<RequiredKeys<StoredSchema>, keyof StoredSchema>>;

declare const stored: StoredSchema;
const schema: string = stored.schema;
const updatedAt: string | null = stored.updatedAt;
const builtin: string[] = stored.builtin;
const includesAuth: boolean = builtin.includes('Auth');
void [schema, updatedAt, includesAuth];
