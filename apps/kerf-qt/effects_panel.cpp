#include "effects_panel.h"

#include <QComboBox>
#include <QHBoxLayout>
#include <QImage>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QPainter>
#include <QPixmap>
#include <QSignalBlocker>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>

namespace kerf {

namespace {

constexpr QSize kCard{160, 90};
// Previews asked for at once; the rest wait until the cards narrow.
constexpr int kPreviewLimit = 48;

QPixmap placeholder(const QString &category) {
    QPixmap pixmap(kCard);
    pixmap.fill(QColor(0x2a, 0x2a, 0x2e));
    QPainter painter(&pixmap);
    painter.setPen(QColor(0x8a, 0x8a, 0x92));
    painter.drawText(pixmap.rect(), Qt::AlignCenter, category);
    return pixmap;
}

} // namespace

EffectsPanel::EffectsPanel(QWidget *parent) : QWidget(parent) {
    setObjectName(QStringLiteral("effectsPanel"));
    setAttribute(Qt::WA_StyledBackground);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(8, 8, 8, 8);
    layout->setSpacing(6);
    auto *filters = new QHBoxLayout;
    m_category = new QComboBox;
    m_category->setObjectName(QStringLiteral("effectsCategory"));
    m_search = new QLineEdit;
    m_search->setObjectName(QStringLiteral("effectsSearch"));
    m_search->setPlaceholderText(QStringLiteral("Find an effect: glow, zoom, retro…"));
    m_search->setClearButtonEnabled(true);
    filters->addWidget(m_category);
    filters->addWidget(m_search, 1);
    layout->addLayout(filters);
    m_hint = new QLabel;
    m_hint->setObjectName(QStringLiteral("muted"));
    m_hint->setWordWrap(true);
    layout->addWidget(m_hint);
    m_cards = new QListWidget;
    m_cards->setObjectName(QStringLiteral("effectsCards"));
    m_cards->setViewMode(QListView::IconMode);
    m_cards->setIconSize(kCard);
    m_cards->setGridSize(QSize(kCard.width() + 16, kCard.height() + 40));
    m_cards->setResizeMode(QListView::Adjust);
    m_cards->setMovement(QListView::Static);
    m_cards->setWordWrap(true);
    m_cards->setUniformItemSizes(true);
    layout->addWidget(m_cards, 1);
    m_previewTimer = new QTimer(this);
    m_previewTimer->setSingleShot(true);
    m_previewTimer->setInterval(150);
    connect(m_previewTimer, &QTimer::timeout, this, &EffectsPanel::requestPreviews);
    connect(m_category, &QComboBox::currentIndexChanged, this, &EffectsPanel::refilter);
    connect(m_search, &QLineEdit::textChanged, this, &EffectsPanel::refilter);
    connect(m_cards, &QListWidget::itemActivated, this, &EffectsPanel::add);
    setClip(QString(), false);
}

void EffectsPanel::setCatalog(const QJsonArray &catalog) {
    m_cards->clear();
    m_items.clear();
    QStringList categories;
    for (const QJsonValue &value : catalog) {
        const QJsonObject fx = value.toObject();
        if (fx.value(QStringLiteral("kind")).toString() != QStringLiteral("effect")) continue;
        const QString id = fx.value(QStringLiteral("id")).toString();
        const QString category = fx.value(QStringLiteral("category")).toString();
        auto *item = new QListWidgetItem(placeholder(category), fx.value(QStringLiteral("name")).toString());
        item->setData(Qt::UserRole, id);
        item->setData(Qt::UserRole + 1, category);
        item->setData(Qt::UserRole + 2,
                      QStringLiteral("%1 %2 %3").arg(id, category, fx.value(QStringLiteral("doc")).toString()).toLower());
        item->setToolTip(fx.value(QStringLiteral("doc")).toString());
        m_cards->addItem(item);
        m_items.insert(id, item);
        if (!categories.contains(category)) categories << category;
    }
    std::sort(categories.begin(), categories.end());
    const QSignalBlocker blocker(m_category);
    m_category->clear();
    m_category->addItem(QStringLiteral("All effects"));
    m_category->addItems(categories);
    refilter();
}

void EffectsPanel::setClip(const QString &clip, bool picture) {
    if (clip == m_clip && picture == m_picture) return;
    m_clip = clip;
    m_picture = picture;
    for (QListWidgetItem *item : std::as_const(m_items))
        item->setIcon(placeholder(item->data(Qt::UserRole + 1).toString()));
    m_hint->setText(clip.isEmpty() ? QStringLiteral("Select a clip to see effects on it, then double-click one to add it.")
                    : picture      ? QStringLiteral("Double-click an effect to add it to the clip.")
                                   : QStringLiteral("Double-click an effect to add it to the clip. Cards show media clips."));
    m_previewTimer->start();
}

void EffectsPanel::setPreview(const QString &clip, const QString &fx, const QImage &image) {
    if (clip != m_clip || image.isNull()) return;
    if (QListWidgetItem *item = m_items.value(fx))
        item->setIcon(QPixmap::fromImage(image.scaled(kCard, Qt::KeepAspectRatio, Qt::SmoothTransformation)));
}

void EffectsPanel::refilter() {
    const QString category = m_category->currentIndex() > 0 ? m_category->currentText() : QString();
    const QStringList words = m_search->text().toLower().split(QLatin1Char(' '), Qt::SkipEmptyParts);
    for (QListWidgetItem *item : std::as_const(m_items)) {
        const QString text = item->data(Qt::UserRole + 2).toString();
        const bool shown = (category.isEmpty() || item->data(Qt::UserRole + 1).toString() == category) &&
                           std::all_of(words.begin(), words.end(), [&text](const QString &word) { return text.contains(word); });
        item->setHidden(!shown);
    }
    m_previewTimer->start();
}

QStringList EffectsPanel::shownEffects() const {
    QStringList shown;
    for (int row = 0; row < m_cards->count() && shown.size() < kPreviewLimit; ++row)
        if (!m_cards->item(row)->isHidden()) shown << m_cards->item(row)->data(Qt::UserRole).toString();
    return shown;
}

void EffectsPanel::requestPreviews() {
    if (!m_clip.isEmpty() && m_picture) emit previewsWanted(m_clip, shownEffects());
}

void EffectsPanel::add(QListWidgetItem *item) {
    if (item && !m_clip.isEmpty()) emit addRequested(item->data(Qt::UserRole).toString());
}

} // namespace kerf
