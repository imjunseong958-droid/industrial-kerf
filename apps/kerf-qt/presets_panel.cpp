#include "presets_panel.h"

#include <QAction>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QLabel>
#include <QListWidget>
#include <QPushButton>
#include <QToolButton>
#include <QVBoxLayout>

namespace kerf {

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

} // namespace

PresetsPanel::PresetsPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("presetsPanel"));
    setAttribute(Qt::WA_StyledBackground);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(6, 6, 6, 6);
    layout->setSpacing(6);
    auto *header = new QHBoxLayout;
    header->setContentsMargins(2, 0, 0, 0);
    auto *title = new QLabel(QStringLiteral("SHARED IN presets/"));
    title->setObjectName(QStringLiteral("presetsTitle"));
    title->setToolTip(QStringLiteral("Presets in the project's presets/ folder travel with its repository"));
    header->addWidget(title);
    header->addStretch(1);
    m_refresh = new QToolButton;
    m_refresh->setObjectName(QStringLiteral("presetsRefresh"));
    header->addWidget(m_refresh);
    layout->addLayout(header);
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("presetList"));
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setTextElideMode(Qt::ElideRight);
    layout->addWidget(m_list, 1);
    m_empty = new QLabel(QStringLiteral("No presets yet. Save the program sequence as a preset, "
                                        "or sync to get your collaborators'."));
    m_empty->setObjectName(QStringLiteral("muted"));
    m_empty->setWordWrap(true);
    m_empty->setAlignment(Qt::AlignCenter);
    layout->addWidget(m_empty, 1);
    m_list->hide();
    auto *footer = new QHBoxLayout;
    footer->setSpacing(6);
    m_apply = new QPushButton(QStringLiteral("Apply…"));
    m_apply->setObjectName(QStringLiteral("presetApply"));
    m_apply->setToolTip(QStringLiteral("Choose this project's media for the preset's slots, then review it as a proposal"));
    footer->addWidget(m_apply);
    footer->addStretch(1);
    m_save = new QToolButton;
    m_save->setObjectName(QStringLiteral("presetSave"));
    m_save->setToolButtonStyle(Qt::ToolButtonTextOnly);
    footer->addWidget(m_save);
    layout->addLayout(footer);
    const auto apply = [this] {
        const QListWidgetItem *item = m_list->currentItem();
        if (m_apply->isEnabled() && item) emit applyRequested(item->data(kPathRole).toString());
    };
    connect(m_apply, &QPushButton::clicked, this, apply);
    connect(m_list, &QListWidget::itemActivated, this, apply);
    connect(m_list, &QListWidget::currentItemChanged, this, &PresetsPanel::updateButtons);
    updateButtons();
}

void PresetsPanel::setActions(QAction *save, QAction *refresh) {
    m_save->setDefaultAction(save);
    // Refresh Presets has no menu item; this button is its only face.
    refresh->setIconText(QStringLiteral("↻"));
    m_refresh->setDefaultAction(refresh);
}

void PresetsPanel::setConnected(bool connected) {
    m_connected = connected;
    updateButtons();
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
        QStringList details;
        QStringList tip{relative};
        if (!error.isEmpty()) {
            details << QStringLiteral("Cannot be read: %1").arg(error);
            tip << error;
        } else {
            const QJsonArray mediaSlots = preset.value(QStringLiteral("slots")).toArray();
            details << count(mediaSlots.size(), QStringLiteral("1 slot"), QStringLiteral("%1 slots"));
            const qsizetype texts = preset.value(QStringLiteral("texts")).toObject().size();
            if (texts > 0) details << count(texts, QStringLiteral("1 text"), QStringLiteral("%1 texts"));
            for (const QJsonValue &slot : mediaSlots) tip << slotLine(slot.toObject());
        }
        const QString author = preset.value(QStringLiteral("author")).toString();
        if (!author.isEmpty()) details << author;
        const QDateTime updated = QDateTime::fromString(preset.value(QStringLiteral("updated")).toString(), Qt::ISODate);
        if (updated.isValid()) details << updated.toLocalTime().toString(QStringLiteral("d MMM yyyy"));
        auto *item = new QListWidgetItem(QStringLiteral("%1\n%2").arg(name, details.join(QStringLiteral(" · "))));
        item->setData(kPathRole, path);
        item->setData(kReadableRole, error.isEmpty());
        item->setToolTip(tip.join(QLatin1Char('\n')));
        if (!error.isEmpty()) item->setForeground(QColor("#8a6f72"));
        m_list->addItem(item);
        if (path == current) m_list->setCurrentItem(item);
    }
    m_list->setVisible(m_list->count() > 0);
    m_empty->setVisible(m_list->count() == 0);
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

} // namespace kerf
