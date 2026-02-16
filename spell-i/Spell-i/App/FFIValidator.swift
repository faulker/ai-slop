import Foundation

enum FFIValidator {
    static func validate() {
        print("FFI Validation: Starting...")
        
        let engine = SpellEngine()
        let text = "This is a tset"
        print("FFI Validation: Linting text: '\(text)'")
        
        let results = engine.lint_text(text)
        let count = results.count()
        print("FFI Validation: Found \(count) lints")
        
        var foundTset = false
        for i in 0..<count {
            let idx = UInt(i)
            let message = results.message(idx).toString()
            let errorType = results.error_type(idx).toString()
            let start = results.start_offset(idx)
            let end = results.end_offset(idx)
            
            let originalWord: String
            if let startIdx = text.utf8.index(text.utf8.startIndex, offsetBy: Int(start), limitedBy: text.utf8.endIndex),
               let endIdx = text.utf8.index(text.utf8.startIndex, offsetBy: Int(end), limitedBy: text.utf8.endIndex) {
                originalWord = String(text.utf8[startIdx..<endIdx]) ?? "???"
            } else {
                originalWord = "???"
            }
            
            print("Linter Result [\(i)]: \(errorType) - \(message) at \(start):\(end) (\(originalWord))")
            
            if originalWord == "tset" {
                foundTset = true
                let suggCount = results.suggestion_count(idx)
                print("  Suggestions (\(suggCount)):")
                for j in 0..<suggCount {
                    print("    - \(results.suggestion(idx, UInt(j)).toString())")
                }
            }
        }
        
        if foundTset {
            print("FFI Validation: SUCCESS")
            exit(0)
        } else {
            print("FFI Validation: FAILURE - 'tset' not flagged")
            exit(1)
        }
    }
}
