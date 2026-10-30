#include "project_panel.h"
#include "editor_common.h"
#include "media_artwork.h"

#include <QFontMetrics>
#include <QJsonObject>
#include <QListWidget>
#include <QPainter>
#include <QVBoxLayout>

#include <utility>

namespace kerf {

namespace {

constexpr QSize kThumbnail{112, 63};

const QString kOffline = QStringLiteral("OFFLINE");
const QString kNotDownloaded = QStringLiteral("NOT DOWNLOADED");
const QString kExternal = QStringLiteral("EXTERNAL");

// A thumbnail letterboxed onto a fixed tile, so the bin keeps a grid, with
// the media's badge in its corner.
QIcon tile(const QImage &frame, bool audio, const QString &badge) {
    QPixmap pixmap(kThumbnail);
    pixmap.fill(QColor("#0e0e0e"));
    QPainter painter(&pixmap);
    if (!frame.isNull() && badge != kOffline) {
        const QImage scaled = frame.scaled(kThumbnail, Qt::KeepAspectRatio, Qt::SmoothTransformation);
        painter.drawImage((kThumbnail.width() - scaled.width()) / 2, (kThumbnail.height() - scaled.height()) / 2, scaled);
    } else {
        painter.setPen(QColor("#6f6f6f"));
        painter.drawText(pixmap.rect(), Qt::AlignCenter, audio ? QStringLiteral("AUDIO") : QStringLiteral("MEDIA"));
    }
    if (!badge.isEmpty()) {
        QFont font = painter.font();
        font.setPixelSize(8);
        font.setBold(true);
        painter.setFont(font);
        const QRect text = QFontMetrics(font).boundingRect(badge);
        const QRectF box(4, 4, text.width() + 8, 13);
        const QColor fill = badge == kOffline ? QColor("#5a262c") : badge == kNotDownloaded ? QColor("#3a3220") : QColor("#2e2e2e");
        const QColor ink = badge == kOffline ? QColor("#f3b9be") : badge == kNotDownloaded ? QColor("#f0b429") : QColor("#cfcfcf");
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        painter.setBrush(fill);
        painter.drawRoundedRect(box, 3, 3);
        painter.setPen(ink);
        painter.drawText(box, Qt::AlignCenter, badge);
    }
    return QIcon(pixmap);
}

QString badgeTip(const QString &badge) {
    if (badge == kOffline) return QStringLiteral("Offline: the file is missing. Replace Media… points this slot at another file.");
    if (badge == kNotDownloaded) return QStringLiteral("Not downloaded: a Git LFS file that a sync downloads.");
    if (badge == kExternal) return QStringLiteral("Outside the project folder, so collaborators do not have it. Consolidate Media copies it into media/.");
    return {};
}

} // namespace

ProjectPanel::ProjectPanel(MediaArtwork *artwork, QWidget *parent) : QWidget(parent), m_artwork(artwork) {
    setObjectName(QStringLiteral("projectPanel"));
    setAttribute(Qt::WA_StyledBackground);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(4, 4, 4, 4);
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("projectBin"));
    m_list->setViewMode(QListView::IconMode);
    m_list->setIconSize(kThumbnail);
    m_list->setGridSize(QSize(kThumbnail.width() + 16, kThumbnail.height() + 46));
    m_list->setTextElideMode(Qt::ElideMiddle);
    m_list->setResizeMode(QListView::Adjust);
    m_list->setMovement(QListView::Static);
    m_list->setWordWrap(true);
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
    layout->addWidget(m_list);
    connect(m_list, &QListWidget::itemActivated, this, [this](QListWidgetItem *item) {
        emit openRequested(item->data(Qt::UserRole).toString());
    });
    connect(m_list, &QListWidget::customContextMenuRequested, this, [this](const QPoint &at) {
        QListWidgetItem *item = m_list->itemAt(at);
        if (item) m_list->setCurrentItem(item);
        emit menuRequested(item ? item->data(Qt::UserRole).toString() : QString(), m_list->viewport()->mapToGlobal(at));
    });
    if (m_artwork) connect(m_artwork, &MediaArtwork::changed, this, &ProjectPanel::refresh);
}

void ProjectPanel::setSources(const QJsonArray &sources) {
    if (sources == m_sources) return;
    m_sources = sources;
    refresh();
}

void ProjectPanel::setMediaStatus(const QJsonObject &status) {
    QHash<QString, QString> badges;
    // A missing file outranks the rest: it cannot be downloaded or consolidated.
    for (const auto &[key, badge] : {std::pair{QStringLiteral("external"), kExternal},
                                     std::pair{QStringLiteral("pointers"), kNotDownloaded},
                                     std::pair{QStringLiteral("missing"), kOffline}})
        for (const QJsonValue &id : status.value(key).toArray()) badges.insert(id.toString(), badge);
    if (badges == m_badges) return;
    m_badges = badges;
    refresh();
}

void ProjectPanel::refresh() {
    const QString current = m_list->currentItem() ? m_list->currentItem()->data(Qt::UserRole).toString() : QString();
    m_list->clear();
    for (const QJsonValue &value : m_sources) {
        const QJsonObject source = value.toObject();
        const QString id = source.value(QStringLiteral("id")).toString();
        const bool audioOnly = !source.contains(QStringLiteral("width"));
        const QString badge = m_badges.value(id);
        auto *item = new QListWidgetItem(tile(m_artwork ? m_artwork->frames.value(id) : QImage(), audioOnly, badge),
                                         QStringLiteral("%1\n%2").arg(sourceName(source),
                                                                      prettyTime(source.value(QStringLiteral("duration")).toDouble())));
        item->setData(Qt::UserRole, id);
        QString tip = QStringLiteral("%1\nDouble-click to open in the Source monitor").arg(source.value(QStringLiteral("path")).toString());
        if (!badge.isEmpty()) tip += QStringLiteral("\n") + badgeTip(badge);
        item->setToolTip(tip);
        m_list->addItem(item);
        if (id == current) m_list->setCurrentItem(item);
    }
}

} // namespace kerf
