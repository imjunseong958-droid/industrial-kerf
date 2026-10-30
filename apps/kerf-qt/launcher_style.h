#pragma once

#include <QColor>
#include <QString>

#include <utility>

namespace kerf {

// The project launcher follows the system appearance rather than the editor's
// dark finishing-room look: macOS's light and dark window colours, with the
// system accent for selection, switches, and the default button. Painted
// parts (tiles, sidebar symbols, switches) read the same tokens.
struct LauncherTheme {
    bool dark = false;
    QColor accent;
    QColor accentPressed;
    QColor onAccent;
    QColor content;
    QColor sidebar;
    QColor sidebarDivider;
    QColor text;
    QColor secondary;
    QColor disabledText;
    QColor separator;
    QColor field;
    QColor fieldBorder;
    QColor fieldDisabled;
    QColor focusRing;
    QColor textSelection;
    QColor selection;
    QColor button;
    QColor buttonBorder;
    QColor buttonPressed;
    QColor buttonDisabled;
    QColor group;
    QColor groupBorder;
    QColor tile;
    QColor tileGlyph;
    QColor badge;
    QColor switchOff;
    QColor knob;
    QColor error;
    QColor success;
    QColor scrollHandle;
    QColor tooltip;
};

// `over` laid on `under` at `amount` (0 to 1).
inline QColor launcherMix(const QColor &under, const QColor &over, qreal amount) {
    return QColor::fromRgbF(under.redF() + (over.redF() - under.redF()) * amount,
                            under.greenF() + (over.greenF() - under.greenF()) * amount,
                            under.blueF() + (over.blueF() - under.blueF()) * amount);
}

inline LauncherTheme launcherTheme(bool dark, const QColor &accent) {
    LauncherTheme t;
    t.dark = dark;
    t.accent = accent;
    t.accentPressed = accent.darker(dark ? 120 : 115);
    t.onAccent = QColor("#ffffff");
    t.focusRing = accent;
    t.focusRing.setAlphaF(0.35);
    if (dark) {
        t.content = QColor("#1e1e1e");
        t.sidebar = QColor("#262628");
        t.sidebarDivider = QColor("#3a3a3c");
        t.text = QColor("#f5f5f7");
        t.secondary = QColor("#98989d");
        t.disabledText = QColor("#6e6e73");
        t.separator = QColor("#38383a");
        t.field = QColor("#2c2c2e");
        t.fieldBorder = QColor("#48484a");
        t.fieldDisabled = QColor("#252527");
        t.selection = QColor(255, 255, 255, 26);
        t.button = QColor("#3a3a3c");
        t.buttonBorder = QColor("#4a4a4c");
        t.buttonPressed = QColor("#4a4a4c");
        t.buttonDisabled = QColor("#2a2a2c");
        t.group = QColor("#252527");
        t.groupBorder = QColor("#38383a");
        t.tile = QColor("#4a4c51");
        t.badge = QColor("#6e7076");
        t.switchOff = QColor("#48484a");
        t.knob = QColor("#e8e8ed");
        t.error = QColor("#ff6961");
        t.success = QColor("#30d158");
        t.scrollHandle = QColor(255, 255, 255, 64);
        t.tooltip = QColor("#2c2c2e");
    } else {
        t.content = QColor("#ffffff");
        t.sidebar = QColor("#f2f2f4");
        t.sidebarDivider = QColor("#d8d8dc");
        t.text = QColor("#1d1d1f");
        t.secondary = QColor("#6e6e73");
        t.disabledText = QColor("#9a9a9f");
        t.separator = QColor("#e3e3e6");
        t.field = QColor("#ffffff");
        t.fieldBorder = QColor("#c9c9ce");
        t.fieldDisabled = QColor("#f5f5f7");
        t.selection = QColor(0, 0, 0, 20);
        t.button = QColor("#ffffff");
        t.buttonBorder = QColor("#c9c9ce");
        t.buttonPressed = QColor("#ebebed");
        t.buttonDisabled = QColor("#f5f5f7");
        t.group = QColor("#f6f6f8");
        t.groupBorder = QColor("#e3e3e6");
        t.tile = QColor("#5e6168");
        t.badge = QColor("#8e9196");
        t.switchOff = QColor("#d9d9de");
        t.knob = QColor("#ffffff");
        t.error = QColor("#d70015");
        t.success = QColor("#248a3d");
        t.scrollHandle = QColor(0, 0, 0, 64);
        t.tooltip = QColor("#f5f5f7");
    }
    t.tileGlyph = QColor("#ffffff");
    t.textSelection = launcherMix(t.field, accent, dark ? 0.45 : 0.28);
    return t;
}

// A complete style sheet for the launcher and everything in it. The editor's
// application style sheet stays in force for every property this one leaves
// out, so each widget type here sets all it relies on.
inline QString launcherStyleSheet(const LauncherTheme &t) {
    const auto c = [](const QColor &color) {
        return color.alpha() == 255 ? color.name() : color.name(QColor::HexArgb);
    };
    QString sheet = QStringLiteral(R"QSS(
* {
    color: @text;
    font-size: 13px;
}
QDialog#projectLauncher,
QWidget#launcherContent,
QMessageBox { background: @content; }
QWidget#launcherPagesArea,
QWidget#launcherPages,
QWidget#launcherDetails,
QWidget#launcherDetailsBody,
QWidget#presetSlots { background: transparent; border: 0; }
QToolTip {
    color: @text;
    background: @tooltip;
    border: 1px solid @fieldBorder;
    border-radius: 4px;
    padding: 4px 7px;
    font-size: 12px;
}

QLabel { background: transparent; border: 0; padding: 0; }
QLabel#launcherHeading { font-size: 15px; font-weight: 600; }
QLabel#launcherHint { color: @secondary; font-size: 12px; }
QLabel#launcherEyebrow { color: @secondary; font-size: 12px; font-weight: 600; }
QLabel#launcherProblem { color: @secondary; font-size: 12px; }
QLabel#launcherProblem[state="error"] { color: @error; }
QLabel#launcherDetailsSummary { color: @secondary; font-size: 12px; }
QLabel#launcherFormLabel { padding-top: 1px; }
QLabel#presetSlotName { font-weight: 600; }
QLabel[fit="none"] { color: @secondary; font-size: 12px; }
QLabel[fit="yes"] { color: @success; font-size: 12px; }
QLabel[fit="no"] { color: @error; font-size: 12px; }

QListWidget,
QListView {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    outline: 0;
    selection-background-color: transparent;
    selection-color: @text;
}
QListWidget#launcherSidebar {
    background: @sidebar;
    border: 0;
    border-right: 1px solid @sidebarDivider;
    padding: 10px 8px;
}
QListWidget#presetChoices {
    background: @group;
    border: 1px solid @groupBorder;
    border-radius: 8px;
    padding: 0;
}
QFrame#presetSlotGroup {
    background: @group;
    border: 1px solid @groupBorder;
    border-radius: 8px;
}
QFrame#presetSlot { background: transparent; border: 0; }
QFrame#launcherGroupSeparator {
    background: @separator;
    border: 0;
    margin-left: 14px;
    min-height: 1px;
    max-height: 1px;
}
QWidget#launcherDetailsRow {
    background: transparent;
    border: 0;
    border-top: 1px solid @separator;
}
QFrame#launcherFooter {
    background: @content;
    border: 0;
    border-top: 1px solid @separator;
}

QScrollArea { background: transparent; border: 0; }
QScrollArea > QWidget > QWidget { background: transparent; }
QScrollBar:vertical,
QScrollBar:horizontal { background: transparent; border: 0; margin: 2px; }
QScrollBar:vertical { width: 10px; }
QScrollBar:horizontal { height: 10px; }
QScrollBar::handle:vertical,
QScrollBar::handle:horizontal { background: @scrollHandle; border: 0; border-radius: 3px; min-height: 28px; min-width: 28px; }
QScrollBar::add-line,
QScrollBar::sub-line { width: 0; height: 0; border: 0; background: transparent; }
QScrollBar::add-page,
QScrollBar::sub-page { background: transparent; }

QToolButton#launcherDetailsToggle {
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 0;
    min-height: 0;
    color: @text;
    font-size: 13px;
}

QPushButton {
    background: @button;
    color: @text;
    border: 1px solid @buttonBorder;
    border-radius: 6px;
    padding: 2px 14px;
    min-height: 18px;
    font-size: 13px;
    font-weight: 400;
    outline: 0;
}
QPushButton:pressed { background: @buttonPressed; }
QPushButton:disabled { color: @disabledText; background: @buttonDisabled; border-color: @separator; }
QPushButton#launcherChoose {
    background: @accent;
    border-color: @accent;
    color: @onAccent;
    min-width: 64px;
}
QPushButton#launcherChoose:pressed { background: @accentPressed; border-color: @accentPressed; }
QPushButton#launcherChoose:disabled { color: @disabledText; background: @buttonDisabled; border-color: @separator; }

QLineEdit,
QDoubleSpinBox,
QComboBox {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 2px 8px;
    min-height: 18px;
    max-height: 18px;
    selection-background-color: @textSelection;
    selection-color: @text;
    placeholder-text-color: @disabledText;
}
QLineEdit:focus,
QDoubleSpinBox:focus,
QComboBox:focus { border-color: @accent; }
QLineEdit:disabled,
QDoubleSpinBox:disabled,
QComboBox:disabled { color: @disabledText; background: @fieldDisabled; border-color: @separator; }
QComboBox { padding-left: 10px; padding-right: 24px; }
QComboBox::drop-down {
    subcontrol-origin: padding;
    subcontrol-position: center right;
    width: 22px;
    border: 0;
    background: transparent;
}
QComboBox::down-arrow { image: none; border: 0; width: 0; height: 0; }
QComboBox QAbstractItemView {
    background: @field;
    color: @text;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
    outline: 0;
    selection-background-color: @accent;
    selection-color: @onAccent;
}
QComboBox QAbstractItemView::item { min-height: 22px; padding: 0 8px; border: 0; border-radius: 4px; }
QAbstractSpinBox::up-button,
QAbstractSpinBox::down-button { width: 0; border: 0; background: transparent; }
QCheckBox { background: transparent; spacing: 8px; }

QProgressBar {
    background: @separator;
    border: 0;
    border-radius: 2px;
    min-height: 4px;
    max-height: 4px;
}
QProgressBar::chunk { background: @accent; border-radius: 2px; }

QMenu {
    background: @field;
    border: 1px solid @fieldBorder;
    border-radius: 6px;
    padding: 4px;
}
QMenu::item { color: @text; background: transparent; padding: 3px 18px 3px 10px; border-radius: 4px; }
QMenu::item:selected { color: @onAccent; background: @accent; }
QMenu::item:disabled { color: @disabledText; background: transparent; }
QMenu::separator { height: 1px; background: @separator; margin: 4px 6px; }
)QSS");
    // Longer names first, so @accent does not eat @accentPressed.
    const std::pair<const char *, QColor> tokens[] = {
        {"@accentPressed", t.accentPressed}, {"@onAccent", t.onAccent}, {"@accent", t.accent},
        {"@content", t.content}, {"@sidebarDivider", t.sidebarDivider}, {"@sidebar", t.sidebar},
        {"@textSelection", t.textSelection}, {"@text", t.text}, {"@secondary", t.secondary},
        {"@disabledText", t.disabledText}, {"@separator", t.separator}, {"@fieldBorder", t.fieldBorder},
        {"@fieldDisabled", t.fieldDisabled}, {"@field", t.field}, {"@buttonBorder", t.buttonBorder},
        {"@buttonPressed", t.buttonPressed}, {"@buttonDisabled", t.buttonDisabled}, {"@button", t.button},
        {"@groupBorder", t.groupBorder}, {"@group", t.group}, {"@error", t.error}, {"@success", t.success},
        {"@scrollHandle", t.scrollHandle}, {"@tooltip", t.tooltip},
    };
    for (const auto &[token, color] : tokens) sheet.replace(QLatin1String(token), c(color));
    return sheet;
}

} // namespace kerf
