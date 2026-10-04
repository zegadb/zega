import pkg from '../package.json'

/** The package manifest is the single source for the server release used by both npm and zega. */
export const SERVER_VERSION = pkg.optionalDependencies['@zegadb/server-darwin-arm64']

if (
    SERVER_VERSION === undefined ||
    Object.values(pkg.optionalDependencies).length !== 4 ||
    Object.values(pkg.optionalDependencies).some((version) => version !== SERVER_VERSION)
) {
    throw new Error('all four @zegadb/server platform packages must use the same version')
}
