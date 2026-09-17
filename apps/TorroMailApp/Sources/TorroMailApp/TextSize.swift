import AppKit
import SwiftUI
import TorroMailKit

private struct TextScaleKey: EnvironmentKey {
    static let defaultValue: CGFloat = CGFloat(TextSize.defaultPercent) / 100
}

extension EnvironmentValues {
    var textScale: CGFloat {
        get { self[TextScaleKey.self] }
        set { self[TextScaleKey.self] = newValue }
    }
}

/// macOS semantic fonts do not grow with Dynamic Type. Resolve their native
/// point sizes explicitly, preserving weight and monospaced variants.
private struct ScaledFont: ViewModifier {
    @Environment(\.textScale) private var scale
    let style: Font.TextStyle
    let weight: Font.Weight?
    let design: Font.Design

    func body(content: Content) -> some View {
        content.font(Self.font(style, scale: scale, weight: weight, design: design))
    }

    static func font(
        _ style: Font.TextStyle, scale: CGFloat,
        weight: Font.Weight? = nil, design: Font.Design = .default
    ) -> Font {
        let nativeStyle: NSFont.TextStyle
        switch style {
        case .largeTitle: nativeStyle = .largeTitle
        case .title: nativeStyle = .title1
        case .title2: nativeStyle = .title2
        case .title3: nativeStyle = .title3
        case .headline: nativeStyle = .headline
        case .subheadline: nativeStyle = .subheadline
        case .callout: nativeStyle = .callout
        case .footnote: nativeStyle = .footnote
        case .caption: nativeStyle = .caption1
        case .caption2: nativeStyle = .caption2
        default: nativeStyle = .body
        }
        return .system(
            size: NSFont.preferredFont(forTextStyle: nativeStyle).pointSize * scale,
            weight: weight ?? (style == .headline ? .bold : .regular),
            design: design
        )
    }
}

private struct PreferredTextSize: ViewModifier {
    @AppStorage(TextSize.storageKey) private var percent = TextSize.defaultPercent

    func body(content: Content) -> some View {
        let scale = CGFloat(TextSize.normalized(percent)) / 100
        content
            .environment(\.textScale, scale)
            .font(ScaledFont.font(.body, scale: scale))
    }
}

private struct ReadableContentWidth: ViewModifier {
    @Environment(\.textScale) private var scale

    func body(content: Content) -> some View {
        content.frame(maxWidth: 720 * max(1, scale), alignment: .topLeading)
    }
}

private struct ScaledPadding: ViewModifier {
    @Environment(\.textScale) private var scale
    let edges: Edge.Set
    let length: CGFloat

    func body(content: Content) -> some View {
        content.padding(edges, length * scale)
    }
}

private struct ScaledSymbolFont: ViewModifier {
    @Environment(\.textScale) private var scale
    let size: CGFloat
    let weight: Font.Weight

    func body(content: Content) -> some View {
        content.font(.system(size: size * scale, weight: weight))
    }
}

extension View {
    func scaledFont(
        _ style: Font.TextStyle, weight: Font.Weight? = nil,
        design: Font.Design = .default
    ) -> some View {
        modifier(ScaledFont(style: style, weight: weight, design: design))
    }

    func preferredTextSize() -> some View {
        modifier(PreferredTextSize())
    }

    func readableContentWidth() -> some View {
        modifier(ReadableContentWidth())
    }

    func scaledPadding(_ length: CGFloat) -> some View {
        modifier(ScaledPadding(edges: .all, length: length))
    }

    func scaledPadding(_ edges: Edge.Set, _ length: CGFloat) -> some View {
        modifier(ScaledPadding(edges: edges, length: length))
    }

    func scaledSymbolFont(size: CGFloat, weight: Font.Weight = .regular) -> some View {
        modifier(ScaledSymbolFont(size: size, weight: weight))
    }
}

struct TextSizeCommands: Commands {
    @AppStorage(TextSize.storageKey) private var percent = TextSize.defaultPercent

    var body: some Commands {
        // Use the existing View menu, not a second menu with the same name.
        CommandGroup(before: .sidebar) {
            Button(L("Larger Text")) { percent = TextSize.larger(than: percent) }
                .keyboardShortcut("+", modifiers: .command)
                .disabled(TextSize.normalized(percent) == TextSize.steps.last)
            Button(L("Smaller Text")) { percent = TextSize.smaller(than: percent) }
                .keyboardShortcut("-", modifiers: .command)
                .disabled(TextSize.normalized(percent) == TextSize.steps.first)
            Button(L("Default Text Size")) { percent = TextSize.defaultPercent }
                .keyboardShortcut("0", modifiers: .command)
            Divider()
        }
    }
}

struct TextSizeSection: View {
    @AppStorage(TextSize.storageKey) private var percent = TextSize.defaultPercent

    var body: some View {
        Section {
            Picker(L("Text size"), selection: Binding(
                get: { TextSize.normalized(percent) },
                set: { percent = $0 }
            )) {
                ForEach(TextSize.steps, id: \.self) { step in
                    Text(step == TextSize.defaultPercent
                         ? String(format: L("%d%% (Default)"), step)
                         : "\(step)%")
                        .tag(step)
                }
            }
        } header: {
            Text(L("Appearance"))
        } footer: {
            Text(L("Change text size with ⌘+ and ⌘−. ⌘0 restores the default size."))
                .scaledFont(.caption)
        }
    }
}

/// Native sidebar labels otherwise substitute their own unscaled font.
struct ScaledSidebarLabelStyle: LabelStyle {
    @Environment(\.textScale) private var scale

    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 8 * scale) {
            configuration.icon
                .font(.system(size: 13 * scale))
                .frame(width: 18 * scale)
            configuration.title
                .scaledFont(.body)
        }
    }
}
