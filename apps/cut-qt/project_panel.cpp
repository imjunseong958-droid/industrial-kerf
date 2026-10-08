#include "project_panel.h"
#include "editor_common.h"
#include "media_artwork.h"

#include <QJsonObject>
#include <QListWidget>
#include <QPainter>
#include <QVBoxLayout>

namespace cut {

namespace {

constexpr QSize kThumbnail{112, 63};

// A thumbnail letterboxed onto a fixed tile, so the bin keeps a grid.
QIcon tile(const QImage &frame, bool audio) {
    QPixmap pixmap(kThumbnail);
    pixmap.fill(QColor("#0e0e0e"));
    QPainter painter(&pixmap);
    if (!frame.isNull()) {
        const QImage scaled = frame.scaled(kThumbnail, Qt::KeepAspectRatio, Qt::SmoothTransformation);
        painter.drawImage((kThumbnail.width() - scaled.width()) / 2, (kThumbnail.height() - scaled.height()) / 2, scaled);
    } else {
        painter.setPen(QColor("#6f6f6f"));
        painter.drawText(pixmap.rect(), Qt::AlignCenter, audio ? QStringLiteral("AUDIO") : QStringLiteral("MEDIA"));
    }
    return QIcon(pixmap);
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
    layout->addWidget(m_list);
    connect(m_list, &QListWidget::itemActivated, this, [this](QListWidgetItem *item) {
        emit openRequested(item->data(Qt::UserRole).toString());
    });
    if (m_artwork) connect(m_artwork, &MediaArtwork::changed, this, &ProjectPanel::refresh);
}

void ProjectPanel::setSources(const QJsonArray &sources) {
    if (sources == m_sources) return;
    m_sources = sources;
    refresh();
}

void ProjectPanel::refresh() {
    const QString current = m_list->currentItem() ? m_list->currentItem()->data(Qt::UserRole).toString() : QString();
    m_list->clear();
    for (const QJsonValue &value : m_sources) {
        const QJsonObject source = value.toObject();
        const QString id = source.value(QStringLiteral("id")).toString();
        const bool audioOnly = !source.contains(QStringLiteral("width"));
        auto *item = new QListWidgetItem(tile(m_artwork ? m_artwork->frames.value(id) : QImage(), audioOnly),
                                         QStringLiteral("%1\n%2").arg(sourceName(source),
                                                                      prettyTime(source.value(QStringLiteral("duration")).toDouble())));
        item->setData(Qt::UserRole, id);
        item->setToolTip(QStringLiteral("%1\nDouble-click to open in the Source monitor").arg(source.value(QStringLiteral("path")).toString()));
        m_list->addItem(item);
        if (id == current) m_list->setCurrentItem(item);
    }
}

} // namespace cut
