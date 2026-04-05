import XCTest
@testable import txtmem

final class DatabaseManagerTests: XCTestCase {
    override func setUp() {
        super.setUp()
        // Use in-memory database to avoid polluting production data
        DatabaseManager.shared.dbPathOverride = ":memory:"
        DatabaseManager.shared.initialize()
    }

    func testCreateAndFetchCategory() {
        let cat = DatabaseManager.shared.createCategory(name: "TestCat_\(UUID().uuidString.prefix(6))")
        XCTAssertNotNil(cat)

        let categories = DatabaseManager.shared.fetchCategories()
        XCTAssertTrue(categories.contains(where: { $0.id == cat?.id }))
    }

    func testCreateAndFetchEntry() {
        let entry = DatabaseManager.shared.createEntry(text: "Test entry \(UUID().uuidString.prefix(6))")
        XCTAssertNotNil(entry)
        XCTAssertFalse(entry!.isSent)
        XCTAssertNil(entry!.categoryId)

        let uncategorized = DatabaseManager.shared.fetchUncategorizedEntries()
        XCTAssertTrue(uncategorized.contains(where: { $0.id == entry?.id }))
    }

    func testMoveEntry() {
        let cat = DatabaseManager.shared.createCategory(name: "MoveCat_\(UUID().uuidString.prefix(6))")!
        let entry = DatabaseManager.shared.createEntry(text: "Move me \(UUID().uuidString.prefix(6))")!

        let moved = DatabaseManager.shared.moveEntry(id: entry.id, toCategoryId: cat.id)
        XCTAssertTrue(moved)

        let catEntries = DatabaseManager.shared.fetchEntries(categoryId: cat.id)
        XCTAssertTrue(catEntries.contains(where: { $0.id == entry.id }))
    }

    func testMarkAsSent() {
        let entry = DatabaseManager.shared.createEntry(text: "Send me \(UUID().uuidString.prefix(6))")!
        let marked = DatabaseManager.shared.markEntryAsSent(id: entry.id)
        XCTAssertTrue(marked)

        let entries = DatabaseManager.shared.fetchEntries()
        let updated = entries.first(where: { $0.id == entry.id })
        XCTAssertTrue(updated?.isSent ?? false)
    }

    func testToggleSent() {
        let entry = DatabaseManager.shared.createEntry(text: "Toggle me \(UUID().uuidString.prefix(6))")!

        _ = DatabaseManager.shared.toggleEntrySent(id: entry.id, isSent: true)
        var entries = DatabaseManager.shared.fetchEntries()
        XCTAssertTrue(entries.first(where: { $0.id == entry.id })?.isSent ?? false)

        _ = DatabaseManager.shared.toggleEntrySent(id: entry.id, isSent: false)
        entries = DatabaseManager.shared.fetchEntries()
        XCTAssertFalse(entries.first(where: { $0.id == entry.id })?.isSent ?? true)
    }

    func testDeleteEntry() {
        let entry = DatabaseManager.shared.createEntry(text: "Delete me \(UUID().uuidString.prefix(6))")!
        let deleted = DatabaseManager.shared.deleteEntry(id: entry.id)
        XCTAssertTrue(deleted)

        let entries = DatabaseManager.shared.fetchEntries()
        XCTAssertFalse(entries.contains(where: { $0.id == entry.id }))
    }

    func testClearCompleted() {
        let e1 = DatabaseManager.shared.createEntry(text: "Sent1_\(UUID().uuidString.prefix(6))")!
        let e2 = DatabaseManager.shared.createEntry(text: "Sent2_\(UUID().uuidString.prefix(6))")!
        _ = DatabaseManager.shared.createEntry(text: "NotSent_\(UUID().uuidString.prefix(6))")

        _ = DatabaseManager.shared.markEntryAsSent(id: e1.id)
        _ = DatabaseManager.shared.markEntryAsSent(id: e2.id)

        let cleared = DatabaseManager.shared.clearCompletedEntries()
        XCTAssertEqual(cleared, 2)
    }

    func testDeleteCategoryMoveToUncategorized() {
        let cat = DatabaseManager.shared.createCategory(name: "DelCat_\(UUID().uuidString.prefix(6))")!
        let entry = DatabaseManager.shared.createEntry(text: "Orphan me \(UUID().uuidString.prefix(6))", categoryId: cat.id)!

        DatabaseManager.shared.deleteCategory(id: cat.id, moveToUncategorized: true)

        let entries = DatabaseManager.shared.fetchUncategorizedEntries()
        XCTAssertTrue(entries.contains(where: { $0.id == entry.id }))
    }
}
