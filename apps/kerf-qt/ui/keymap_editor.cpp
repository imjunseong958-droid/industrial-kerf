#include "keymap_editor.h"

#include "bridge.h"
#include "command_registry.h"
#include "components.h"
#include "design_tokens.h"
#include "tray_shell.h"

#include <QAction>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeySequenceEdit>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QPainter>
#include <QSettings>
#include <QStyledItemDelegate>
#include <QVBoxLayout>

#include <algorithm>
#include <utility>

namespace kerf {

namespace {

using tokens::color;

constexpr int kIdRole = Qt::UserRole;
constexpr int kWhereRole = Qt::UserRole + 1;
constexpr int kKeysRole = Qt::UserRole + 2;
constexpr int kCustomRole = Qt::UserRole + 3;
constexpr int kPad = 20;

// shell::text's ink and size, kept under the app's sheet (its `*` rule
// gives every widget the text colour and 12 px).
void setInk(QLabel *label, const tokens::TypeStyle &style, QRgb ink) {
    label->setStyleSheet(QStringLiteral("QLabel { color: %1; font-size: %2px; font-weight: %3; background: transparent; }")
                             .arg(tokens::color(ink).name(QColor::HexRgb))
                             .arg(style.px)
                             .arg(style.weight));
}

QLabel *inked(const QString &content, const tokens::TypeStyle &style, QRgb ink) {
    QLabel *label = shell::text(content, style, ink);
    setInk(label, style, ink);
    return label;
}

QString nativeText(const QList<QKeySequence> &keys) {
    QStringList shown;
    for (const QKeySequence &key : keys)
        if (!key.isEmpty()) shown << key.toString(QKeySequence::NativeText);
    return shown.join(QStringLiteral("  "));
}

QString portableText(const QList<QKeySequence> &keys) {
    QStringList text;
    for (const QKeySequence &key : keys)
        if (!key.isEmpty()) text << key.toString(QKeySequence::PortableText);
    return text.join(QLatin1Char('\n'));
}

QList<QKeySequence> fromPortable(const QString &text) {
    QList<QKeySequence> keys;
    for (const QString &part : text.split(QLatin1Char('\n'), Qt::SkipEmptyParts)) {
        const QKeySequence key = QKeySequence::fromString(part, QKeySequence::PortableText);
        if (!key.isEmpty()) keys << key;
    }
    return keys;
}

// Where a command without a menu acts, by its scope and its id.
QString whereOf(const QJsonObject &command) {
    const QString menu = command.value(QStringLiteral("menu")).toString();
    if (!menu.isEmpty()) return menu;
    const QString scope = command.value(QStringLiteral("scope")).toString().toLower();
    if (scope == QStringLiteral("timeline")) return QStringLiteral("Timeline");
    if (scope == QStringLiteral("source")) return QStringLiteral("Source");
    const QString area = command.value(QStringLiteral("id")).toString().section(QLatin1Char('.'), 0, 0);
    static const QHash<QString, QString> areas{
        {QStringLiteral("bin"), QStringLiteral("Bin")},           {QStringLiteral("browser"), QStringLiteral("Browser")},
        {QStringLiteral("composer"), QStringLiteral("Comments")}, {QStringLiteral("comment"), QStringLiteral("Comments")},
        {QStringLiteral("source"), QStringLiteral("Source")},     {QStringLiteral("sequence"), QStringLiteral("Sequence")},
        {QStringLiteral("tool"), QStringLiteral("Timeline")},     {QStringLiteral("clip"), QStringLiteral("Clip")},
        {QStringLiteral("playback"), QStringLiteral("Playback")}, {QStringLiteral("view"), QStringLiteral("View")},
        {QStringLiteral("viewer"), QStringLiteral("View")},       {QStringLiteral("proposal"), QStringLiteral("Comments")},
    };
    return areas.value(area, QStringLiteral("Window"));
}

// A row (28 tall, radius 6, pad 0 8): the command in UI 12 ink, where it
// acts in 11 ink3, and its keys right-aligned in mono 11 ink2 (accent ink
// when they are one's own); the chosen row on raised.
class RowDelegate final : public QStyledItemDelegate {
public:
    using QStyledItemDelegate::QStyledItemDelegate;
    QSize sizeHint(const QStyleOptionViewItem &option, const QModelIndex &) const override {
        return QSize(option.rect.width(), KeymapEditor::kRow);
    }
    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        const QRectF row = QRectF(option.rect).adjusted(0, 0, 0, 0);
        const bool selected = option.state & QStyle::State_Selected;
        const bool hovered = option.state & QStyle::State_MouseOver;
        if (selected || hovered) {
            painter->setPen(Qt::NoPen);
            painter->setBrush(color(selected ? tokens::on : tokens::raised));
            painter->drawRoundedRect(row, tokens::radius::control, tokens::radius::control);
        }
        const QRectF inner = row.adjusted(8, 0, -8, 0);
        const QString keys = index.data(kKeysRole).toString();
        const bool custom = index.data(kCustomRole).toBool();
        painter->setFont(tokens::font(tokens::monoSmall));
        const QString shownKeys = keys.isEmpty() ? QStringLiteral("—") : keys;
        const int keysWidth = ui::snapWidth(tokens::monoSmall, shownKeys);
        painter->setPen(color(keys.isEmpty() ? tokens::dis : custom ? tokens::accentInk : tokens::ink2));
        painter->drawText(QRectF(inner.right() - keysWidth, inner.top(), keysWidth, inner.height()), Qt::AlignRight | Qt::AlignVCenter,
                          shownKeys);
        const qreal room = inner.width() - keysWidth - 16;
        const QString name = index.data(Qt::DisplayRole).toString();
        const QString where = index.data(kWhereRole).toString();
        painter->setFont(tokens::font(tokens::control));
        const QFontMetricsF nameMetrics(painter->font());
        const QString shownName = nameMetrics.elidedText(name, Qt::ElideRight, room);
        painter->setPen(color(tokens::ink));
        painter->drawText(QRectF(inner.left(), inner.top(), room, inner.height()), Qt::AlignLeft | Qt::AlignVCenter, shownName);
        const qreal whereLeft = inner.left() + nameMetrics.horizontalAdvance(shownName) + 8;
        if (whereLeft < inner.left() + room) {
            painter->setFont(tokens::font(tokens::meta));
            painter->setPen(color(tokens::ink3));
            const QString shownWhere =
                QFontMetricsF(painter->font()).elidedText(where, Qt::ElideRight, inner.left() + room - whereLeft);
            painter->drawText(QRectF(whereLeft, inner.top(), inner.left() + room - whereLeft, inner.height()),
                              Qt::AlignLeft | Qt::AlignVCenter, shownWhere);
        }
        painter->restore();
    }
};

} // namespace

// ================================================================ KeymapCustomiser

KeymapCustomiser::KeymapCustomiser(CommandRegistry *registry, QObject *parent) : QObject(parent), m_registry(registry) {
    setObjectName(QStringLiteral("keymapCustomiser"));
    load();
    if (registry)
        connect(registry, &CommandRegistry::shortcutsChanged, this, [this] {
            // A keymap chosen (the registry reloaded its keys): one's own
            // keys for it go back on, before the rest hear of the change.
            if (m_applying) return;
            const bool other = m_registry && m_registry->keymap() != m_keymap;
            if (other) load();
            apply();
            // The editor lists the new keymap's keys, own ones or not.
            if (other) emit changed();
        });
    apply();
}

QString KeymapCustomiser::settingsGroup(const QString &keymap) {
    return QStringLiteral("commands/custom/%1").arg(keymap.isEmpty() ? QStringLiteral("kerf") : keymap);
}

void KeymapCustomiser::load() {
    m_commands.clear();
    m_index.clear();
    m_keymap = m_registry ? m_registry->keymap() : QString();
    char *raw = kerf_commands_catalog(m_keymap.toUtf8().constData());
    if (!raw) return;
    const QJsonObject catalog = QJsonDocument::fromJson(QByteArray(raw)).object();
    kerf_bridge_string_free(raw);
    if (m_keymap.isEmpty()) m_keymap = catalog.value(QStringLiteral("keymap")).toString();
    for (const QJsonValue &value : catalog.value(QStringLiteral("commands")).toArray()) {
        const QJsonObject command = value.toObject();
        const QString id = command.value(QStringLiteral("id")).toString();
        const QJsonArray keys = command.value(QStringLiteral("keys")).toArray();
        const bool listed = !command.value(QStringLiteral("menu")).toString().isEmpty() || !keys.isEmpty() ||
                            command.value(QStringLiteral("kind")).toString() == QStringLiteral("frontend");
        if (id.isEmpty() || !listed || (m_registry && !m_registry->action(id))) continue;
        Command entry;
        entry.id = id;
        entry.label = command.value(QStringLiteral("label")).toString();
        entry.where = whereOf(command);
        entry.scope = command.value(QStringLiteral("scope")).toString().toLower();
        for (const QJsonValue &key : keys) {
            const QKeySequence sequence = CommandRegistry::keymapKey(key.toString());
            if (!sequence.isEmpty()) entry.defaults << sequence;
        }
        m_index.insert(id, int(m_commands.size()));
        m_commands << entry;
    }
}

KeymapCustomiser::Command KeymapCustomiser::command(const QString &id) const {
    const int at = m_index.value(id, -1);
    return at >= 0 ? m_commands.at(at) : Command{};
}

QList<QKeySequence> KeymapCustomiser::keys(const QString &id) const {
    const QAction *action = m_registry ? m_registry->action(id) : nullptr;
    return action ? action->shortcuts() : QList<QKeySequence>{};
}

QList<QKeySequence> KeymapCustomiser::defaults(const QString &id) const {
    return command(id).defaults;
}

bool KeymapCustomiser::isCustom(const QString &id) const {
    return QSettings().contains(QStringLiteral("%1/%2").arg(settingsGroup(m_keymap), id));
}

bool KeymapCustomiser::scopesOverlap(const QString &a, const QString &b) {
    const auto window = [](const QString &scope) { return scope.isEmpty() || scope == QStringLiteral("window"); };
    return a == b || window(a) || window(b);
}

QStringList KeymapCustomiser::conflicts(const QString &id, const QKeySequence &key) const {
    QStringList taken;
    if (key.isEmpty()) return taken;
    const QString scope = command(id).scope;
    for (const Command &other : m_commands) {
        if (other.id == id || !scopesOverlap(scope, other.scope)) continue;
        if (keys(other.id).contains(key)) taken << other.id;
    }
    return taken;
}

void KeymapCustomiser::store(const QString &id, const QList<QKeySequence> &keys) {
    QSettings settings;
    const QString key = QStringLiteral("%1/%2").arg(settingsGroup(m_keymap), id);
    // Keys the keymap gives anyway are not one's own.
    if (keys == defaults(id)) settings.remove(key);
    else settings.setValue(key, portableText(keys));
}

void KeymapCustomiser::setKeys(const QString &id, const QList<QKeySequence> &keys) {
    QAction *action = m_registry ? m_registry->action(id) : nullptr;
    if (!action) return;
    if (action->shortcuts() != keys) action->setShortcuts(keys);
    // The tip the registry writes: the label with the first key, and why
    // it cannot run when it cannot.
    const QString reason = m_registry->reason(id);
    const QString tip = m_registry->labelWithShortcut(id);
    action->setToolTip(reason.isEmpty() ? tip : QStringLiteral("%1\n%2").arg(tip, reason));
}

void KeymapCustomiser::announce() {
    if (m_registry) {
        // Everything that names a key follows the registry's signal.
        m_applying = true;
        emit m_registry->shortcutsChanged();
        m_applying = false;
    }
    emit changed();
}

void KeymapCustomiser::apply() {
    if (!m_registry) return;
    QSettings settings;
    settings.beginGroup(settingsGroup(m_keymap));
    const QStringList ids = settings.childKeys();
    for (const QString &id : ids)
        if (has(id)) setKeys(id, fromPortable(settings.value(id).toString()));
    settings.endGroup();
    if (!ids.isEmpty()) announce();
}

void KeymapCustomiser::assign(const QString &id, const QKeySequence &key) {
    if (!has(id) || key.isEmpty()) return;
    for (const QString &other : conflicts(id, key)) {
        QList<QKeySequence> left = keys(other);
        left.removeAll(key);
        store(other, left);
        setKeys(other, left);
    }
    store(id, {key});
    setKeys(id, {key});
    announce();
}

void KeymapCustomiser::clear(const QString &id) {
    if (!has(id)) return;
    store(id, {});
    setKeys(id, {});
    announce();
}

void KeymapCustomiser::reset(const QString &id) {
    if (!has(id)) return;
    QSettings().remove(QStringLiteral("%1/%2").arg(settingsGroup(m_keymap), id));
    setKeys(id, defaults(id));
    announce();
}

void KeymapCustomiser::resetAll() {
    QSettings().remove(settingsGroup(m_keymap));
    for (const Command &command : std::as_const(m_commands)) setKeys(command.id, command.defaults);
    announce();
}

// ================================================================ KeymapEditor

KeymapEditor::KeymapEditor(KeymapCustomiser *customiser, QWidget *parent) : QDialog(parent), m_customiser(customiser) {
    setObjectName(QStringLiteral("keymapEditor"));
    setWindowTitle(QStringLiteral("Customise Keyboard"));
    resize(kWidth, kHeight);
    setMinimumSize(420, 420);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(kPad, kPad, kPad, kPad);
    layout->setSpacing(tokens::space::s12);

    // The title and the keymap the keys are one's own over.
    auto *header = new QVBoxLayout;
    header->setSpacing(0);
    header->addWidget(inked(QStringLiteral("Keyboard shortcuts"), tokens::title, tokens::ink));
    m_meta = inked(QString(), tokens::meta, tokens::ink3);
    m_meta->setObjectName(QStringLiteral("keymapEditorMeta"));
    header->addWidget(m_meta);
    layout->addLayout(header);

    m_search = new QLineEdit;
    m_search->setObjectName(QStringLiteral("keymapSearch"));
    m_search->setPlaceholderText(QStringLiteral("Search commands and keys"));
    m_search->setAccessibleName(m_search->placeholderText());
    m_search->setClearButtonEnabled(true);
    m_search->setFont(tokens::font(tokens::control));
    layout->addWidget(m_search);

    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("keymapList"));
    m_list->setItemDelegate(new RowDelegate(m_list));
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setUniformItemSizes(true);
    m_list->setMouseTracking(true);
    m_list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setSelectionMode(QAbstractItemView::SingleSelection);
    m_list->setStyleSheet(QStringLiteral("QListWidget { background: %1; border: none; outline: none; border-radius: 8px; padding: 4px; }")
                              .arg(color(tokens::well).name()));
    layout->addWidget(m_list, 1);

    // The chosen command: its name, the recorder, Clear and Reset to default.
    auto *editRow = new QHBoxLayout;
    editRow->setSpacing(tokens::space::s8);
    m_chosen = inked(QString(), tokens::controlStrong, tokens::ink);
    m_chosen->setObjectName(QStringLiteral("keymapChosen"));
    m_chosen->setMinimumWidth(120);
    editRow->addWidget(m_chosen, 1);
    m_recorder = new QKeySequenceEdit;
    m_recorder->setObjectName(QStringLiteral("keymapRecorder"));
    m_recorder->setMaximumSequenceLength(1);
    m_recorder->setFixedWidth(140);
    m_recorder->setFont(tokens::font(tokens::monoSmall));
    m_recorder->setAccessibleName(QStringLiteral("New shortcut"));
    ui::setTip(m_recorder, QStringLiteral("Click, then press the new shortcut"));
    editRow->addWidget(m_recorder);
    m_clear = new ui::GhostButton(QStringLiteral("Clear"));
    m_clear->setObjectName(QStringLiteral("keymapClear"));
    m_clear->setTip(QStringLiteral("No shortcut for this command"));
    editRow->addWidget(m_clear);
    m_reset = new ui::GhostButton(QStringLiteral("Reset to default"));
    m_reset->setObjectName(QStringLiteral("keymapReset"));
    editRow->addWidget(m_reset);
    layout->addLayout(editRow);

    // A key another command has: what it is, and whether to move it here.
    m_warnRow = new QWidget;
    m_warnRow->setObjectName(QStringLiteral("keymapWarnRow"));
    auto *warnLayout = new QHBoxLayout(m_warnRow);
    warnLayout->setContentsMargins(0, 0, 0, 0);
    warnLayout->setSpacing(tokens::space::s8);
    m_warning = inked(QString(), tokens::meta, tokens::warn);
    m_warning->setObjectName(QStringLiteral("keymapWarning"));
    m_warning->setWordWrap(true);
    warnLayout->addWidget(m_warning, 1);
    m_keepIt = new ui::GhostButton(QStringLiteral("Cancel"));
    m_keepIt->setObjectName(QStringLiteral("keymapKeep"));
    m_useHere = new ui::Button(QStringLiteral("Use here"), ui::Button::Kind::Primary);
    m_useHere->setObjectName(QStringLiteral("keymapUseHere"));
    warnLayout->addWidget(m_keepIt);
    warnLayout->addWidget(m_useHere);
    m_warnRow->hide();
    layout->addWidget(m_warnRow);

    auto *footer = new QHBoxLayout;
    footer->setSpacing(tokens::space::s8);
    m_resetAll = new ui::GhostButton(QStringLiteral("Reset all"));
    m_resetAll->setObjectName(QStringLiteral("keymapResetAll"));
    m_resetAll->setTip(QStringLiteral("Every command back to the keymap's own keys"));
    footer->addWidget(m_resetAll);
    footer->addStretch(1);
    m_done = new ui::Button(QStringLiteral("Done"), ui::Button::Kind::Primary);
    m_done->setObjectName(QStringLiteral("keymapDone"));
    footer->addWidget(m_done);
    layout->addLayout(footer);

    connect(m_search, &QLineEdit::textChanged, this, &KeymapEditor::setFilter);
    connect(m_list, &QListWidget::currentItemChanged, this, [this] {
        cancelConflict();
        syncEditor();
    });
    connect(m_recorder, &QKeySequenceEdit::editingFinished, this, [this] {
        if (m_recording) return;
        const QKeySequence key = m_recorder->keySequence();
        if (!key.isEmpty()) record(key);
    });
    connect(m_clear, &QAbstractButton::clicked, this, [this] {
        cancelConflict();
        if (m_customiser) m_customiser->clear(selectedCommand());
    });
    connect(m_reset, &QAbstractButton::clicked, this, [this] {
        cancelConflict();
        if (m_customiser) m_customiser->reset(selectedCommand());
    });
    connect(m_useHere, &QAbstractButton::clicked, this, &KeymapEditor::confirmConflict);
    connect(m_keepIt, &QAbstractButton::clicked, this, &KeymapEditor::cancelConflict);
    connect(m_resetAll, &QAbstractButton::clicked, this, [this] {
        cancelConflict();
        if (m_customiser) m_customiser->resetAll();
    });
    connect(m_done, &QAbstractButton::clicked, this, &QDialog::accept);
    if (m_customiser) connect(m_customiser, &KeymapCustomiser::changed, this, &KeymapEditor::refreshRows);
    rebuild();
}

void KeymapEditor::rebuild() {
    const QString keep = selectedCommand();
    m_list->clear();
    if (m_customiser) {
        QString label;
        if (CommandRegistry *registry = m_customiser->registry())
            for (const QJsonValue &value : registry->keymaps())
                if (value.toObject().value(QStringLiteral("id")).toString() == m_customiser->keymap())
                    label = value.toObject().value(QStringLiteral("label")).toString();
        m_meta->setText(QStringLiteral("%1 keymap · changes apply at once and are kept on this Mac")
                            .arg(label.isEmpty() ? m_customiser->keymap() : label));
        for (const KeymapCustomiser::Command &command : m_customiser->commands()) {
            auto *item = new QListWidgetItem(command.label, m_list);
            item->setData(kIdRole, command.id);
            item->setData(kWhereRole, command.where);
            item->setSizeHint(QSize(0, kRow));
        }
    }
    refreshRows();
    if (!keep.isEmpty()) selectCommand(keep);
    else if (m_list->count() > 0) m_list->setCurrentRow(0);
    setFilter(m_search->text());
}

void KeymapEditor::refreshRows() {
    if (m_customiser && m_customiser->keymap() != m_meta->property("keymap").toString()) {
        // Another keymap was chosen while the editor is open.
        m_meta->setProperty("keymap", m_customiser->keymap());
        if (m_list->count() != m_customiser->commands().size()) {
            rebuild();
            return;
        }
    }
    for (int row = 0; row < m_list->count(); ++row) {
        QListWidgetItem *item = m_list->item(row);
        const QString id = item->data(kIdRole).toString();
        item->setData(kKeysRole, keyText(id));
        item->setData(kCustomRole, m_customiser && m_customiser->isCustom(id));
    }
    m_list->viewport()->update();
    syncEditor();
}

QString KeymapEditor::keyText(const QString &id) const {
    return m_customiser ? nativeText(m_customiser->keys(id)) : QString();
}

void KeymapEditor::setFilter(const QString &text) {
    if (m_search->text() != text) m_search->setText(text);
    const QStringList words = text.toLower().split(QLatin1Char(' '), Qt::SkipEmptyParts);
    for (int row = 0; row < m_list->count(); ++row) {
        QListWidgetItem *item = m_list->item(row);
        const QString haystack = QStringLiteral("%1 %2 %3 %4")
                                     .arg(item->text(), item->data(kWhereRole).toString(), item->data(kIdRole).toString(),
                                          item->data(kKeysRole).toString())
                                     .toLower();
        item->setHidden(!std::all_of(words.cbegin(), words.cend(), [&haystack](const QString &word) { return haystack.contains(word); }));
    }
    // The chosen command stays chosen while it is shown; else the first shown.
    QListWidgetItem *current = m_list->currentItem();
    if (!current || current->isHidden())
        for (int row = 0; row < m_list->count(); ++row)
            if (!m_list->item(row)->isHidden()) {
                m_list->setCurrentRow(row);
                break;
            }
}

QStringList KeymapEditor::shownCommands() const {
    QStringList shown;
    for (int row = 0; row < m_list->count(); ++row)
        if (!m_list->item(row)->isHidden()) shown << m_list->item(row)->data(kIdRole).toString();
    return shown;
}

void KeymapEditor::selectCommand(const QString &id) {
    for (int row = 0; row < m_list->count(); ++row)
        if (m_list->item(row)->data(kIdRole).toString() == id) {
            m_list->setCurrentRow(row);
            m_list->scrollToItem(m_list->item(row));
            return;
        }
}

QString KeymapEditor::selectedCommand() const {
    const QListWidgetItem *item = m_list->currentItem();
    return item && !item->isHidden() ? item->data(kIdRole).toString() : QString();
}

void KeymapEditor::syncEditor() {
    const QString id = selectedCommand();
    const bool chosen = !id.isEmpty() && m_customiser;
    m_chosen->setText(chosen ? m_list->currentItem()->text() : QStringLiteral("Choose a command"));
    m_recording = true;
    const QList<QKeySequence> keys = chosen ? m_customiser->keys(id) : QList<QKeySequence>{};
    m_recorder->setKeySequence(keys.value(0));
    m_recording = false;
    m_recorder->setEnabled(chosen);
    m_clear->setEnabled(chosen && !keys.isEmpty());
    const bool custom = chosen && m_customiser->isCustom(id);
    m_reset->setEnabled(custom);
    const QString defaults = chosen ? nativeText(m_customiser->defaults(id)) : QString();
    m_reset->setTip(custom ? QStringLiteral("Back to %1").arg(defaults.isEmpty() ? QStringLiteral("no shortcut") : defaults)
                           : QStringLiteral("This is the keymap's own shortcut"));
}

void KeymapEditor::record(const QKeySequence &key) {
    const QString id = selectedCommand();
    if (!m_customiser || id.isEmpty() || key.isEmpty()) return;
    if (m_customiser->keys(id) == QList<QKeySequence>{key}) {
        cancelConflict();
        return;
    }
    const QStringList taken = m_customiser->conflicts(id, key);
    if (taken.isEmpty()) {
        cancelConflict();
        m_customiser->assign(id, key);
        return;
    }
    QStringList names;
    for (const QString &other : taken) names << QStringLiteral("“%1”").arg(m_customiser->command(other).label);
    m_pendingKey = key;
    m_warning->setText(QStringLiteral("%1 is already %2. Use it here instead?")
                           .arg(key.toString(QKeySequence::NativeText), names.join(QStringLiteral(" and "))));
    m_warnRow->show();
}

QString KeymapEditor::warning() const {
    return m_warnRow->isVisible() || conflictPending() ? m_warning->text() : QString();
}

void KeymapEditor::confirmConflict() {
    if (!conflictPending()) return;
    const QKeySequence key = std::exchange(m_pendingKey, QKeySequence());
    m_warnRow->hide();
    m_warning->clear();
    if (m_customiser) m_customiser->assign(selectedCommand(), key);
}

void KeymapEditor::cancelConflict() {
    if (!conflictPending() && !m_warnRow->isVisible()) return;
    m_pendingKey = QKeySequence();
    m_warnRow->hide();
    m_warning->clear();
    syncEditor();
}

void KeymapEditor::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), color(tokens::card));
}

} // namespace kerf
