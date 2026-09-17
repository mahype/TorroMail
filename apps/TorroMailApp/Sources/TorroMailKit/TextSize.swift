/// App-only typography preference. Percentages refer to the original macOS
/// text sizes; 100% preserves the native layout and is the reset value.
public enum TextSize {
    public static let storageKey = "textSizePercent"
    public static let defaultPercent = 100
    public static let steps = [85, 100, 115, 125, 135, 145, 160]

    public static func normalized(_ percent: Int) -> Int {
        steps.contains(percent) ? percent : defaultPercent
    }

    public static func larger(than percent: Int) -> Int {
        steps.first { $0 > normalized(percent) } ?? steps.last!
    }

    public static func smaller(than percent: Int) -> Int {
        steps.last { $0 < normalized(percent) } ?? steps.first!
    }
}
