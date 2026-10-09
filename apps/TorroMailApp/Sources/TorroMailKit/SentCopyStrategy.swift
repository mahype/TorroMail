import Foundation

public enum SentCopyStrategy: String, Codable, CaseIterable, Hashable, Sendable {
    case imap, provider, none
}
