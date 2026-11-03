#pragma once

// The browser at the right of the bottom band (mockup-spec 8): a 28-point
// strip with the segmented Effects | Titles | Presets and Search (no
// shortcut until a command for it exists),
// over the page chosen: the Effects browser, the Titles page and Presets.
//
// Shell from Q-SKELETON (W1): Effects and Presets are today's panels;
// Titles is disabled until Q-BINBROWSER builds it from the catalog's
// generators in W2 ("Coming in this release"). Q-BINBROWSER fills it in,
// keeping the constructor and this API.

#include <QString>
#include <QWidget>

class QAbstractButton;
class QStackedWidget;
class QToolButton;

namespace kerf {

class EditorBus;
class EffectsPanel;
class PresetsPanel;

class BrowserPanel final : public QWidget {
    Q_OBJECT
public:
    enum class Page { Effects, Titles, Presets };

    explicit BrowserPanel(QWidget *parent, EditorBus *bus);

    void setEffects(EffectsPanel *effects);
    EffectsPanel *effects() const { return m_effects; }
    void setPresets(PresetsPanel *presets);
    PresetsPanel *presets() const { return m_presets; }

    Page page() const { return m_page; }
    // A page not available yet is refused.
    void setPage(Page page);
    static bool pageAvailable(Page page);
    // "effects", "titles", "presets"; and back (Effects for anything else).
    static QString pageId(Page page);
    static Page pageFromId(const QString &id);
    QAbstractButton *pageButton(Page page) const;
    QToolButton *searchButton() const { return m_search; }
    QWidget *currentPage() const;

signals:
    void pageChanged(BrowserPanel::Page page);

protected:
    void paintEvent(QPaintEvent *event) override;

private:
    void showCurrent();

    EditorBus *m_bus = nullptr;
    EffectsPanel *m_effects = nullptr;
    PresetsPanel *m_presets = nullptr;
    QStackedWidget *m_stack = nullptr;
    QWidget *m_titles = nullptr;
    QAbstractButton *m_pageButtons[3] = {};
    QToolButton *m_search = nullptr;
    Page m_page = Page::Effects;
};

} // namespace kerf
