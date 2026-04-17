import Foundation

struct Entry: Identifiable {
    let id: Int64
    var text: String
    var categoryId: Int64?
    var isSent: Bool
    var createdAt: Date
    var updatedAt: Date

    var categoryName: String?
}
