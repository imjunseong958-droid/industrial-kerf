#include "presets_panel.h"
#include "design_tokens.h"
#include "ui/components.h"
#include "ui/tray_shell.h"

#include <QAction>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QListWidget>
#include <QPainter>
#include <QStyledItemDelegate>
#include <QVBoxLayout>

namespace kerf {

using tokens::color;

namespace {

constexpr int kPathRole = Qt::UserRole;
constexpr int kReadableRole = Qt::UserRole + 1;

QString count(qsizetype n, const QString &one, const QString &many) {
    return n == 1 ? one : many.arg(n);
}

// What a slot needs, as the preset dialogs say it.
QString slotLine(const QJsonObject &slot) {
    const bool video = slot.value(QStringLiteral("video")).toBool();
    const bool audio = slot.value(QStringLiteral("audio")).toBool();
    QStringList parts{slot.value(QStringLiteral("id")).toString(),
                      QStringLiteral("%1 s").arg(slot.value(QStringLiteral("seconds")).toDouble(), 0, 'f', 2)};
    if (video || audio)
        parts << (video && audio ? QStringLiteral("picture and sound") : video ? QStringLiteral("picture") : QStringLiteral("sound"));
    const int clips = slot.value(QStringLiteral("clips")).toInt();
    if (clips > 0) parts << count(clips, QStringLiteral("1 clip"), QStringLiteral("%1 clips"));
    return parts.join(QStringLiteral(" · "));
}

// A preset row (mockup .prs .pr): 44 tall, pad 0 12, a bottom line; the
// name in UI 12 ink (an unreadable one in neg) over its meta in UI 11 ink3;
// hover raised, the chosen one on.
class PresetDelegate final : public QStyledItemDelegate {
public:
    explicit PresetDelegate(QListWidget *view) : QStyledItemDelegate(view), m_view(view) {}
    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override {
        return QSize(std::max(1, m_view->viewport()->width()), PresetsPanel::kRowHeight);
    }
    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        painter->save();
        const QRect row = option.rect;
        if (option.state & QStyle::State_Selected) painter->fillRect(row, color(tokens::on));
        else if (option.state & QStyle::State_MouseOver) painter->fillRect(row, color(tokens::raised));
        painter->fillRect(QRect(row.left(), row.bottom(), row.width(), 1), color(tokens::line));
        const QStringList lines = index.data(Qt::DisplayRole).toString().split(QLatin1Char('\n'));
        const QRect text = row.adjusted(tokens::space::s12, 0, -tokens::space::s12, 0);
        const int top = row.top() + (row.height() - tokens::control.line - tokens::meta.line) / 2;
        const QVariant foreground = index.data(Qt::ForegroundRole);
        painter->setFont(tokens::font(tokens::control));
        painter->setPen(foreground.isValid() ? qvariant_cast<QBrush>(foreground).color() : color(tokens::ink));
        painter->drawText(QRect(text.left(), top, text.width(), tokens::control.line), Qt::AlignLeft | Qt::AlignVCenter,
                          QFontMetrics(painter->font()).elidedText(lines.value(0), Qt::ElideRight, text.width()));
        painter->setFont(tokens::font(tokens::meta));
        painter->setPen(color(tokens::ink3));
        painter->drawText(QRect(text.left(), top + tokens::control.line, text.width(), tokens::meta.line), Qt::AlignLeft | Qt::AlignVCenter,
                          QFontMetrics(painter->font()).elidedText(lines.value(1), Qt::ElideRight, text.width()));
        painter->restore();
    }

private:
    QListWidget *m_view;
};

} // namespace

PresetsPanel::PresetsPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("presetsPanel"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    // The category row's place: "Shared presets" and the count.
    auto *header = new QWidget;
    header->setFixedHeight(24 + tokens::space::s4);
    auto *headerLayout = new QHBoxLayout(header);
    headerLayout->setContentsMargins(tokens::space::s12, tokens::space::s4, tokens::space::s12, 0);
    headerLayout->setSpacing(0);
    auto *title = shell::text(QStringLiteral("Shared presets"), tokens::control, tokens::ink2);
    title->setObjectName(QStringLiteral("presetsTitle"));
    title->setToolTip(QStringLiteral("Presets in the project's presets/ folder travel with its repository"));
    headerLayout->addWidget(title);
    headerLayout->addStretch(1);
    m_count = shell::text(QString(), tokens::meta, tokens::ink3);
    m_count->setObjectName(QStringLiteral("presetsCount"));
    headerLayout->addWidget(m_count);
    layout->addWidget(header);
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("presetList"));
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setItemDelegate(new PresetDelegate(m_list));
    m_list->setMouseTracking(true);
    m_list->setUniformItemSizes(true);
    m_list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setStyleSheet(QStringLiteral("QListWidget { background: transparent; border: none; outline: none; }"));
    m_list->viewport()->setAutoFillBackground(false);
    layout->addWidget(m_list, 1);
    m_empty = shell::text(QStringLiteral("No presets yet. Save the program sequence as a preset, or sync to get your collaborators'."),
                          tokens::meta, tokens::ink3);
    m_empty->setObjectName(QStringLiteral("presetsEmpty"));
    m_empty->setWordWrap(true);
    m_empty->setAlignment(Qt::AlignCenter);
    m_empty->setContentsMargins(tokens::space::s12, 0, tokens::space::s12, 0);
    layout->addWidget(m_empty, 1);
    m_list->hide();
    // The footer: pad 12, gap 8.
    auto *footer = new QHBoxLayout;
    footer->setContentsMargins(tokens::space::s12, tokens::space::s12, tokens::space::s12, tokens::space::s12);
    footer->setSpacing(tokens::space::s8);
    m_save = new ui::Button(QStringLiteral("Save current as preset…"));
    m_save->setObjectName(QStringLiteral("presetSave"));
    footer->addWidget(m_save);
    m_apply = new ui::Button(QStringLiteral("Apply…"));
    m_apply->setObjectName(QStringLiteral("presetApply"));
    footer->addWidget(m_apply);
    footer->addStretch(1);
    m_refresh = new ui::IconButton(QStringLiteral("g-reset"));
    m_refresh->setObjectName(QStringLiteral("presetsRefresh"));
    m_refresh->setTip(QStringLiteral("Refresh presets"));
    footer->addWidget(m_refresh);
    layout->addLayout(footer);
    const auto apply = [this] {
        const QListWidgetItem *item = m_list->currentItem();
        if (m_apply->isEnabled() && item) emit applyRequested(item->data(kPathRole).toString());
    };
    connect(m_apply, &QAbstractButton::clicked, this, apply);
    connect(m_list, &QListWidget::itemActivated, this, apply);
    connect(m_list, &QListWidget::currentItemChanged, this, &PresetsPanel::updateButtons);
    connect(m_save, &QAbstractButton::clicked, this, [this] {
        if (m_saveAction && m_saveAction->isEnabled()) m_saveAction->trigger();
    });
    connect(m_refresh, &QAbstractButton::clicked, this, [this] {
        if (m_refreshAction && m_refreshAction->isEnabled()) m_refreshAction->trigger();
    });
    followActions();
    updateButtons();
}

void PresetsPanel::setActions(QAction *save, QAction *refresh) {
    if (m_saveAction) disconnect(m_saveAction, nullptr, this, nullptr);
    if (m_refreshAction) disconnect(m_refreshAction, nullptr, this, nullptr);
    m_saveAction = save;
    m_refreshAction = refresh;
    for (QAction *action : {save, refresh})
        if (action) connect(action, &QAction::changed, this, &PresetsPanel::followActions);
    followActions();
}

void PresetsPanel::followActions() {
    // Enabled as the registry's commands are, with their tips (and reasons).
    shell::followAction(m_save, m_saveAction);
    shell::followAction(m_refresh, m_refreshAction);
}

void PresetsPanel::setConnected(bool connected) {
    m_connected = connected;
    updateButtons();
}

QString PresetsPanel::metaLine(const QJsonObject &preset) {
    const QString error = preset.value(QStringLiteral("error")).toString();
    if (!error.isEmpty()) return QStringLiteral("Cannot be read: %1").arg(error);
    QStringList details;
    const QJsonArray mediaSlots = preset.value(QStringLiteral("slots")).toArray();
    int clips = 0;
    for (const QJsonValue &slot : mediaSlots) clips += slot.toObject().value(QStringLiteral("clips")).toInt();
    if (clips > 0) details << count(clips, QStringLiteral("1 clip"), QStringLiteral("%1 clips"));
    details << count(mediaSlots.size(), QStringLiteral("1 slot"), QStringLiteral("%1 slots"));
    const qsizetype texts = preset.value(QStringLiteral("texts")).toObject().size();
    if (texts > 0) details << count(texts, QStringLiteral("1 text"), QStringLiteral("%1 texts"));
    const QDateTime updated = QDateTime::fromString(preset.value(QStringLiteral("updated")).toString(), Qt::ISODate);
    if (updated.isValid()) details << updated.toLocalTime().toString(QStringLiteral("d MMM"));
    return details.join(QStringLiteral(" · "));
}

void PresetsPanel::setLibrary(const QJsonObject &library, const QString &projectDirectory) {
    const QString current = m_list->currentItem() ? m_list->currentItem()->data(kPathRole).toString() : QString();
    m_list->clear();
    const QDir project(projectDirectory);
    for (const QJsonValue &value : library.value(QStringLiteral("presets")).toArray()) {
        const QJsonObject preset = value.toObject();
        const QString relative = preset.value(QStringLiteral("path")).toString();
        const QString path = project.absoluteFilePath(relative);
        const QString name = preset.value(QStringLiteral("name")).toString(QFileInfo(relative).completeBaseName());
        const QString error = preset.value(QStringLiteral("error")).toString();
        QStringList tip{relative};
        if (!error.isEmpty()) tip << error;
        else
            for (const QJsonValue &slot : preset.value(QStringLiteral("slots")).toArray()) tip << slotLine(slot.toObject());
        const QString author = preset.value(QStringLiteral("author")).toString();
        const QDateTime updated = QDateTime::fromString(preset.value(QStringLiteral("updated")).toString(), Qt::ISODate);
        if (!author.isEmpty() || updated.isValid())
            tip << QStringLiteral("Saved%1%2").arg(author.isEmpty() ? QString() : QStringLiteral(" by ") + author,
                                                  updated.isValid() ? updated.toLocalTime().toString(QStringLiteral(" 'on' d MMM yyyy")) : QString());
        auto *item = new QListWidgetItem(QStringLiteral("%1\n%2").arg(name, metaLine(preset)));
        item->setData(kPathRole, path);
        item->setData(kReadableRole, error.isEmpty());
        item->setToolTip(tip.join(QLatin1Char('\n')));
        if (!error.isEmpty()) item->setForeground(color(tokens::neg));
        m_list->addItem(item);
        if (path == current) m_list->setCurrentItem(item);
    }
    const int presets = m_list->count();
    m_count->setText(presets == 1 ? QStringLiteral("1 item") : QStringLiteral("%1 items").arg(presets));
    m_list->setVisible(presets > 0);
    m_empty->setVisible(presets == 0);
    updateButtons();
}

void PresetsPanel::updateButtons() {
    const QListWidgetItem *item = m_list->currentItem();
    const bool readable = item && item->data(kReadableRole).toBool();
    m_apply->setEnabled(m_connected && readable);
    m_apply->setToolTip(!m_connected ? QStringLiteral("Waiting for the project connection")
                        : !item      ? QStringLiteral("Select a preset")
                        : !readable  ? QStringLiteral("This preset cannot be read")
                                     : QStringLiteral("Choose this project's media for the preset's slots, then review it as a proposal"));
}

void PresetsPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), color(tokens::chrome));
}

} // namespace kerf
