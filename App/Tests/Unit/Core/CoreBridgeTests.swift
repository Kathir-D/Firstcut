// Owner: infra. Proves the Rust core is linked into the app and reachable from Swift.
// Delete nothing here: this is the guard that catches a stale App/Generated or a missing static
// library before it reaches a release.
import Testing

@testable import Firstcut

@Test func coreIsLinkedAndAnswers() {
    #expect(FirstcutCoreBridge.isLinked)
    #expect(FirstcutCoreBridge.greeting.contains("Firstcut core"))
}

@Test func coreVersionIsSemantic() {
    let version = FirstcutCoreBridge.coreVersion
    let parts = version.split(separator: ".")
    #expect(!version.isEmpty)
    #expect((1...3).contains(parts.count))
    #expect(parts.allSatisfy { $0.allSatisfy(\.isNumber) })
}
