import Foundation
import SQLite3

private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

final class DatabaseManager {
    static let shared = DatabaseManager()
    private var db: OpaquePointer?
    private let queue = DispatchQueue(label: "com.txtmem.database")
    private var isOpen = false

    /// Must be set before calling initialize()
    var dbPathOverride: String?

    private static let currentSchemaVersion: Int32 = 1

    private init() {}

    func initialize() {
        queue.sync {
            let dbPath = dbPathOverride ?? getDBPath()
            if sqlite3_open(dbPath, &db) != SQLITE_OK {
                let errMsg = db.flatMap { String(cString: sqlite3_errmsg($0)) } ?? "unknown"
                NSLog("[ThoughtQueue] FATAL: Failed to open database at %@: %@", dbPath, errMsg)
                return
            }
            isOpen = true
            createTables()
            migrateIfNeeded()
        }
    }

    private func getDBPath() -> String {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let appDir = appSupport.appendingPathComponent("ThoughtQueue", isDirectory: true)
        try? FileManager.default.createDirectory(at: appDir, withIntermediateDirectories: true)
        return appDir.appendingPathComponent("thoughtqueue.db").path
    }

    private func createTables() {
        let categoriesSQL = """
            CREATE TABLE IF NOT EXISTS categories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                created_at REAL NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            """
        let entriesSQL = """
            CREATE TABLE IF NOT EXISTS entries (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                category_id INTEGER,
                is_sent INTEGER NOT NULL DEFAULT 0,
                created_at REAL NOT NULL DEFAULT (strftime('%s', 'now')),
                updated_at REAL NOT NULL DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE SET NULL
            );
            """
        executeDDL(categoriesSQL)
        executeDDL(entriesSQL)
    }

    private func migrateIfNeeded() {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "PRAGMA user_version;", -1, &stmt, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(stmt) }

        var version: Int32 = 0
        if sqlite3_step(stmt) == SQLITE_ROW {
            version = sqlite3_column_int(stmt, 0)
        }

        // Future migrations go here:
        // if version < 2 { ... }

        if version < Self.currentSchemaVersion {
            executeDDL("PRAGMA user_version = \(Self.currentSchemaVersion);")
        }
    }

    private func guardOpen() -> Bool {
        if !isOpen {
            NSLog("[ThoughtQueue] ERROR: Database operation attempted but database is not open")
        }
        return isOpen
    }

    @discardableResult
    private func executeDDL(_ sql: String) -> Bool {
        var errMsg: UnsafeMutablePointer<CChar>?
        if sqlite3_exec(db, sql, nil, nil, &errMsg) != SQLITE_OK {
            let msg = errMsg.map { String(cString: $0) } ?? "unknown error"
            sqlite3_free(errMsg)
            NSLog("[ThoughtQueue] SQL error: %@", msg)
            return false
        }
        return true
    }

    private func inTransaction(_ block: () -> Bool) {
        guard executeDDL("BEGIN TRANSACTION;") else { return }
        if block() {
            if !executeDDL("COMMIT;") {
                executeDDL("ROLLBACK;")
            }
        } else {
            executeDDL("ROLLBACK;")
            NSLog("[ThoughtQueue] Transaction rolled back due to failure")
        }
    }

    private func columnText(_ stmt: OpaquePointer?, _ col: Int32) -> String {
        guard let ptr = sqlite3_column_text(stmt, col) else { return "" }
        return String(cString: ptr)
    }

    private func columnTextOrNil(_ stmt: OpaquePointer?, _ col: Int32) -> String? {
        guard sqlite3_column_type(stmt, col) != SQLITE_NULL,
              let ptr = sqlite3_column_text(stmt, col) else { return nil }
        return String(cString: ptr)
    }

    private func bindText(_ stmt: OpaquePointer?, _ index: Int32, _ value: String) {
        sqlite3_bind_text(stmt, index, (value as NSString).utf8String, -1, SQLITE_TRANSIENT)
    }

    // MARK: - Categories

    func createCategory(name: String) -> Category? {
        queue.sync {
            guard guardOpen() else { return nil }
            let sql = "INSERT INTO categories (name) VALUES (?);"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }

            bindText(stmt, 1, name)

            guard sqlite3_step(stmt) == SQLITE_DONE else { return nil }
            let id = sqlite3_last_insert_rowid(db)
            return Category(id: id, name: name, createdAt: Date())
        }
    }

    func fetchCategories() -> [Category] {
        queue.sync {
            guard guardOpen() else { return [] }
            let sql = "SELECT id, name, created_at FROM categories ORDER BY name;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
            defer { sqlite3_finalize(stmt) }

            var categories: [Category] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                let id = sqlite3_column_int64(stmt, 0)
                let name = columnText(stmt, 1)
                let timestamp = sqlite3_column_double(stmt, 2)
                categories.append(Category(id: id, name: name, createdAt: Date(timeIntervalSince1970: timestamp)))
            }
            return categories
        }
    }

    func renameCategory(id: Int64, name: String) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "UPDATE categories SET name = ? WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            bindText(stmt, 1, name)
            sqlite3_bind_int64(stmt, 2, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func deleteCategory(id: Int64, moveToUncategorized: Bool) {
        queue.sync {
            guard guardOpen() else { return }
            inTransaction {
                if moveToUncategorized {
                    let updateSQL = "UPDATE entries SET category_id = NULL, updated_at = strftime('%s', 'now') WHERE category_id = ?;"
                    var stmt: OpaquePointer?
                    guard sqlite3_prepare_v2(db, updateSQL, -1, &stmt, nil) == SQLITE_OK else { return false }
                    sqlite3_bind_int64(stmt, 1, id)
                    let rc = sqlite3_step(stmt)
                    sqlite3_finalize(stmt)
                    guard rc == SQLITE_DONE else { return false }
                } else {
                    let deleteEntriesSQL = "DELETE FROM entries WHERE category_id = ?;"
                    var stmt: OpaquePointer?
                    guard sqlite3_prepare_v2(db, deleteEntriesSQL, -1, &stmt, nil) == SQLITE_OK else { return false }
                    sqlite3_bind_int64(stmt, 1, id)
                    let rc = sqlite3_step(stmt)
                    sqlite3_finalize(stmt)
                    guard rc == SQLITE_DONE else { return false }
                }

                let sql = "DELETE FROM categories WHERE id = ?;"
                var stmt: OpaquePointer?
                guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
                sqlite3_bind_int64(stmt, 1, id)
                let rc = sqlite3_step(stmt)
                sqlite3_finalize(stmt)
                return rc == SQLITE_DONE
            }
        }
    }

    // MARK: - Entries

    func createEntry(text: String, categoryId: Int64? = nil) -> Entry? {
        queue.sync {
            guard guardOpen() else { return nil }
            let sql = "INSERT INTO entries (text, category_id) VALUES (?, ?);"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
            defer { sqlite3_finalize(stmt) }

            bindText(stmt, 1, text)
            if let catId = categoryId {
                sqlite3_bind_int64(stmt, 2, catId)
            } else {
                sqlite3_bind_null(stmt, 2)
            }

            guard sqlite3_step(stmt) == SQLITE_DONE else { return nil }
            let id = sqlite3_last_insert_rowid(db)
            return Entry(id: id, text: text, categoryId: categoryId, isSent: false, createdAt: Date(), updatedAt: Date())
        }
    }

    func fetchEntries(categoryId: Int64? = nil) -> [Entry] {
        queue.sync {
            guard guardOpen() else { return [] }
            let sql: String
            if categoryId != nil {
                sql = """
                    SELECT e.id, e.text, e.category_id, e.is_sent, e.created_at, e.updated_at, c.name
                    FROM entries e LEFT JOIN categories c ON e.category_id = c.id
                    WHERE e.category_id = ?
                    ORDER BY e.created_at DESC;
                    """
            } else {
                sql = """
                    SELECT e.id, e.text, e.category_id, e.is_sent, e.created_at, e.updated_at, c.name
                    FROM entries e LEFT JOIN categories c ON e.category_id = c.id
                    ORDER BY e.created_at DESC;
                    """
            }
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
            defer { sqlite3_finalize(stmt) }

            if let catId = categoryId {
                sqlite3_bind_int64(stmt, 1, catId)
            }

            return collectEntries(from: stmt)
        }
    }

    func fetchUncategorizedEntries() -> [Entry] {
        queue.sync {
            guard guardOpen() else { return [] }
            let sql = """
                SELECT e.id, e.text, e.category_id, e.is_sent, e.created_at, e.updated_at, NULL
                FROM entries e WHERE e.category_id IS NULL
                ORDER BY e.created_at DESC;
                """
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return [] }
            defer { sqlite3_finalize(stmt) }
            return collectEntries(from: stmt)
        }
    }

    func fetchEntriesCount(categoryId: Int64?) -> Int {
        queue.sync {
            guard guardOpen() else { return 0 }
            let sql: String
            if categoryId != nil {
                sql = "SELECT COUNT(*) FROM entries WHERE category_id = ?;"
            } else {
                sql = "SELECT COUNT(*) FROM entries WHERE category_id IS NULL;"
            }
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return 0 }
            defer { sqlite3_finalize(stmt) }

            if let catId = categoryId {
                sqlite3_bind_int64(stmt, 1, catId)
            }

            return sqlite3_step(stmt) == SQLITE_ROW ? Int(sqlite3_column_int(stmt, 0)) : 0
        }
    }

    func fetchTotalEntriesCount() -> Int {
        queue.sync {
            guard guardOpen() else { return 0 }
            let sql = "SELECT COUNT(*) FROM entries;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return 0 }
            defer { sqlite3_finalize(stmt) }
            return sqlite3_step(stmt) == SQLITE_ROW ? Int(sqlite3_column_int(stmt, 0)) : 0
        }
    }

    private func collectEntries(from stmt: OpaquePointer?) -> [Entry] {
        var entries: [Entry] = []
        while sqlite3_step(stmt) == SQLITE_ROW {
            let id = sqlite3_column_int64(stmt, 0)
            let text = columnText(stmt, 1)
            let categoryId: Int64? = sqlite3_column_type(stmt, 2) != SQLITE_NULL ? sqlite3_column_int64(stmt, 2) : nil
            let isSent = sqlite3_column_int(stmt, 3) != 0
            let createdAt = Date(timeIntervalSince1970: sqlite3_column_double(stmt, 4))
            let updatedAt = Date(timeIntervalSince1970: sqlite3_column_double(stmt, 5))
            let categoryName = columnTextOrNil(stmt, 6)
            entries.append(Entry(id: id, text: text, categoryId: categoryId, isSent: isSent, createdAt: createdAt, updatedAt: updatedAt, categoryName: categoryName))
        }
        return entries
    }

    func updateEntry(id: Int64, text: String) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "UPDATE entries SET text = ?, updated_at = strftime('%s', 'now') WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            bindText(stmt, 1, text)
            sqlite3_bind_int64(stmt, 2, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func moveEntry(id: Int64, toCategoryId: Int64?) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "UPDATE entries SET category_id = ?, updated_at = strftime('%s', 'now') WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            if let catId = toCategoryId {
                sqlite3_bind_int64(stmt, 1, catId)
            } else {
                sqlite3_bind_null(stmt, 1)
            }
            sqlite3_bind_int64(stmt, 2, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func toggleEntrySent(id: Int64, isSent: Bool) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "UPDATE entries SET is_sent = ?, updated_at = strftime('%s', 'now') WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            sqlite3_bind_int(stmt, 1, isSent ? 1 : 0)
            sqlite3_bind_int64(stmt, 2, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func markEntryAsSent(id: Int64) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "UPDATE entries SET is_sent = 1, updated_at = strftime('%s', 'now') WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            sqlite3_bind_int64(stmt, 1, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func deleteEntry(id: Int64) -> Bool {
        queue.sync {
            guard guardOpen() else { return false }
            let sql = "DELETE FROM entries WHERE id = ?;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
            defer { sqlite3_finalize(stmt) }

            sqlite3_bind_int64(stmt, 1, id)
            return sqlite3_step(stmt) == SQLITE_DONE
        }
    }

    func clearCompletedEntries() -> Int {
        queue.sync {
            guard guardOpen() else { return 0 }
            let sql = "DELETE FROM entries WHERE is_sent = 1;"
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return 0 }
            defer { sqlite3_finalize(stmt) }
            guard sqlite3_step(stmt) == SQLITE_DONE else { return 0 }
            return Int(sqlite3_changes(db))
        }
    }

    deinit {
        sqlite3_close(db)
    }
}
